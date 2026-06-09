//! Docker 沙箱执行器
//!
//! 将 Bash 命令路由到 Docker 容器中执行，提供硬件级隔离。

use tokio::process::Command;

/// Docker 执行配置
#[derive(Debug, Clone)]
pub struct DockerExecutorConfig {
    /// Docker 镜像名
    pub image: String,
    /// 工作目录在容器中的路径
    pub workdir: String,
    /// 是否自动拉取镜像
    pub auto_pull: bool,
    /// 网络模式: "none", "host", "bridge"
    pub network: String,
    /// 内存限制
    pub memory_limit: String,
    /// CPU 限制
    pub cpu_limit: String,
}

impl Default for DockerExecutorConfig {
    fn default() -> Self {
        Self {
            image: "ubuntu:22.04".into(),
            workdir: "/workspace".into(),
            auto_pull: true,
            network: "none".into(),
            memory_limit: "512m".into(),
            cpu_limit: "1".into(),
        }
    }
}

/// Docker 执行结果
#[derive(Debug)]
pub struct DockerOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Docker 沙箱执行器
pub struct DockerExecutor {
    config: DockerExecutorConfig,
}

impl DockerExecutor {
    pub fn new(config: DockerExecutorConfig) -> Self {
        Self { config }
    }

    /// 检查 Docker 是否可用
    pub async fn is_available() -> bool {
        Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 在 Docker 容器中执行命令
    pub async fn execute(&self, command: &str, timeout_secs: u64) -> crate::Result<DockerOutput> {
        if self.config.auto_pull {
            self.ensure_image().await?;
        }

        let cwd = std::env::current_dir()?;
        let cwd_str = cwd.to_string_lossy();
        let volume_mount = format!("{}:{}", cwd_str, self.config.workdir);

        let mut args: Vec<&str> = vec![
            "run", "--rm",
            "-w", &self.config.workdir,
            "-v", &volume_mount,
            "--network", &self.config.network,
            "--memory", &self.config.memory_limit,
            "--cpus", &self.config.cpu_limit,
        ];

        args.push(&self.config.image);
        args.push("sh");
        args.push("-c");
        args.push(command);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            Command::new("docker").args(&args).output(),
        )
        .await
        .map_err(|_| crate::Error::Agent(format!("Docker command timed out ({}s)", timeout_secs)))?
        .map_err(|e| crate::Error::Agent(format!("Docker exec failed: {}", e)))?;

        Ok(DockerOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    /// 确保镜像已拉取
    async fn ensure_image(&self) -> crate::Result<()> {
        let check = Command::new("docker")
            .args(["image", "inspect", &self.config.image])
            .output()
            .await?;

        if check.status.success() {
            return Ok(());
        }

        tracing::info!(image = %self.config.image, "Pulling Docker image");
        let pull = Command::new("docker")
            .args(["pull", &self.config.image])
            .output()
            .await?;

        if !pull.status.success() {
            let stderr = String::from_utf8_lossy(&pull.stderr);
            return Err(crate::Error::Agent(format!(
                "Failed to pull image '{}': {}",
                self.config.image, stderr
            )));
        }

        Ok(())
    }
}
