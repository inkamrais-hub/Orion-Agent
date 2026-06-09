//! Git 沙箱管理器 — 基于分支的并发工作区隔离
//!
//! 当 Sub-Agent 执行任务时，自动创建 Git 临时分支，
//! 所有文件修改隔离在该分支中。任务完成后合并回主分支，
//! 如果有冲突则交给 ConflictResolverAgent 解决。
//!
//! 如果工作区不在 Git 仓库中，沙箱功能优雅降级（不报错）。

use std::path::PathBuf;

/// Git 沙箱管理器
pub struct GitSandbox {
    working_dir: PathBuf,
    main_branch: String,
}

/// 合并结果
#[derive(Debug)]
pub enum MergeResult {
    /// 合并成功
    Success { branch: String },
    /// 存在冲突
    Conflict { branch: String, files: Vec<String> },
}

impl GitSandbox {
    pub fn new(working_dir: &str) -> Self {
        Self {
            working_dir: PathBuf::from(working_dir),
            main_branch: "main".to_string(),
        }
    }

    /// 设置主分支名 (默认 "main")
    pub fn with_main_branch(mut self, branch: &str) -> Self {
        self.main_branch = branch.to_string();
        self
    }

    /// 创建沙箱分支
    ///
    /// 基于当前主分支创建并切换到新的临时分支。
    /// 分支名格式: `orion_sandbox_{session_id}_{agent_id}`
    pub async fn create_branch(&self, session_id: &str, agent_id: &str) -> crate::Result<String> {
        let branch_name = format!("orion_sandbox_{}_{}", session_id, agent_id);

        // 确保在主分支上
        self.run_git(&["checkout", &self.main_branch]).await?;

        // 创建并切换到新分支
        self.run_git(&["checkout", "-b", &branch_name]).await?;

        tracing::info!(branch = %branch_name, "Sandbox branch created");
        Ok(branch_name)
    }

    /// 合并沙箱分支回主分支
    ///
    /// 尝试将沙箱分支合并到主分支。如果存在冲突，
    /// 返回冲突文件列表供后续处理。
    pub async fn merge_branch(&self, branch_name: &str) -> crate::Result<MergeResult> {
        // 切换回主分支
        self.run_git(&["checkout", &self.main_branch]).await?;

        // 尝试合并
        let output = self.run_git_output(&["merge", branch_name, "--no-edit"]).await?;

        if output.contains("CONFLICT") || output.contains("conflict") {
            // 有冲突，获取冲突文件列表
            let conflicts = self.get_conflict_files().await?;
            Ok(MergeResult::Conflict {
                branch: branch_name.to_string(),
                files: conflicts,
            })
        } else {
            Ok(MergeResult::Success {
                branch: branch_name.to_string(),
            })
        }
    }

    /// 获取冲突文件列表
    async fn get_conflict_files(&self) -> crate::Result<Vec<String>> {
        let output = self
            .run_git_output(&["diff", "--name-only", "--diff-filter=U"])
            .await?;
        Ok(output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// 获取冲突文件内容
    pub async fn get_conflict_content(&self, file_path: &str) -> crate::Result<String> {
        let full_path = self.working_dir.join(file_path);
        tokio::fs::read_to_string(&full_path)
            .await
            .map_err(crate::Error::Io)
    }

    /// 解决冲突后标记为已解决并提交
    pub async fn resolve_and_commit(
        &self,
        file_path: &str,
        resolved_content: &str,
    ) -> crate::Result<()> {
        let full_path = self.working_dir.join(file_path);
        tokio::fs::write(&full_path, resolved_content).await?;
        self.run_git(&["add", file_path]).await?;
        self.run_git(&[
            "commit",
            "-m",
            &format!("resolve conflict in {}", file_path),
        ])
        .await?;
        Ok(())
    }

    /// 删除沙箱分支
    pub async fn cleanup_branch(&self, branch_name: &str) -> crate::Result<()> {
        // 切回主分支再删除，避免删除当前分支失败
        let _ = self.run_git(&["checkout", &self.main_branch]).await;
        let _ = self.run_git(&["branch", "-D", branch_name]).await;
        tracing::info!(branch = %branch_name, "Sandbox branch cleaned up");
        Ok(())
    }

    /// 检查是否在 Git 仓库中
    pub async fn is_git_repo(&self) -> bool {
        self.run_git(&["rev-parse", "--is-inside-work-tree"])
            .await
            .is_ok()
    }

    /// 获取当前分支名
    pub async fn current_branch(&self) -> crate::Result<String> {
        let output = self.run_git_output(&["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        Ok(output.trim().to_string())
    }

    // ── 内部辅助 ─────────────────────────────────────────

    async fn run_git(&self, args: &[&str]) -> crate::Result<()> {
        let output = tokio::process::Command::new("git")
            .current_dir(&self.working_dir)
            .args(args)
            .output()
            .await
            .map_err(|e| crate::Error::Agent(format!("git command failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::Error::Agent(format!(
                "git {:?} failed: {}",
                args, stderr
            )));
        }
        Ok(())
    }

    async fn run_git_output(&self, args: &[&str]) -> crate::Result<String> {
        let output = tokio::process::Command::new("git")
            .current_dir(&self.working_dir)
            .args(args)
            .output()
            .await
            .map_err(|e| crate::Error::Agent(format!("git command failed: {}", e)))?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// 使用 LLM 解决 Git 合并冲突
///
/// 遍历冲突文件列表，调用 LLM 生成合并后的版本，
/// 然后标记为已解决并提交。
pub async fn resolve_conflicts_with_llm(
    sandbox: &GitSandbox,
    conflict_files: &[String],
    provider: &dyn crate::core::provider::Provider,
    model: &str,
) -> crate::Result<()> {
    use crate::core::provider::{ContentBlock, Message, ProviderRequest, Role};

    for file in conflict_files {
        let content = sandbox.get_conflict_content(file).await?;

        let prompt = format!(
            "Resolve the following Git merge conflict. Output ONLY the resolved file content, no explanation.\n\nFile: {}\n\n{}",
            file, content
        );

        let req = ProviderRequest {
            model: model.to_string(),
            messages: vec![Message::new(
                Role::User,
                vec![ContentBlock::Text { text: prompt }],
            )],
            system_prompt: Some(
                "You are a code conflict resolver. Output only the resolved code, no markdown."
                    .to_string(),
            ),
            max_tokens: Some(8192),
            temperature: Some(0.1),
            stream: false,
            tools: None,
            thinking: Some(serde_json::json!({"type": "disabled"})),
            reasoning_effort: None,
            enable_prompt_cache: None,
            cache_key: None,
        };

        let resp = provider.complete(req).await?;
        let resolved: String = resp
            .message
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();

        sandbox.resolve_and_commit(file, &resolved).await?;
        tracing::info!(file = %file, "Conflict resolved by LLM");
    }
    Ok(())
}
