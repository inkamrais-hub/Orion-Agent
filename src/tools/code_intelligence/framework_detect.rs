//! 框架检测 — 自动识别项目使用的框架/语言/构建工具
//!
//! 通过检测标志文件和目录来推断项目类型

use serde::Serialize;
use std::path::Path;

/// 检测到的框架信息
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkInfo {
    /// 语言 (rust, python, javascript, typescript, go, java, c, cpp)
    pub language: String,
    /// 框架名 (nextjs, nuxt, django, spring-boot, express, actix-web, etc.)
    pub frameworks: Vec<String>,
    /// 构建工具 (cargo, npm, yarn, pnpm, maven, gradle, go, pip, poetry, etc.)
    pub build_tool: String,
    /// 包管理器 (cargo, npm, yarn, pnpm, pip, poetry, go, maven, gradle)
    pub package_manager: String,
    /// 运行命令
    pub run_command: String,
    /// 测试命令
    pub test_command: String,
    /// 构建命令
    pub build_command: String,
    /// 项目类型 (cli, library, web-app, api, monorepo)
    pub project_type: String,
    /// 检测到的标志文件
    pub marker_files: Vec<String>,
}

/// 检测项目框架
pub fn detect_framework(root: &Path) -> FrameworkInfo {
    let mut info = FrameworkInfo {
        language: "unknown".into(),
        frameworks: Vec::new(),
        build_tool: "unknown".into(),
        package_manager: "unknown".into(),
        run_command: "unknown".into(),
        test_command: "unknown".into(),
        build_command: "unknown".into(),
        project_type: "unknown".into(),
        marker_files: Vec::new(),
    };

    // ── Rust ──────────────────────────────────────────────
    if has_file(root, "Cargo.toml") {
        info.language = "rust".into();
        info.build_tool = "cargo".into();
        info.package_manager = "cargo".into();
        info.run_command = "cargo run".into();
        info.test_command = "cargo test".into();
        info.build_command = "cargo build --release".into();
        info.marker_files.push("Cargo.toml".into());

        // 检测 Rust 框架
        if has_file(root, "src/main.rs") {
            info.project_type = "cli".into();
        }
        if has_file(root, "src/lib.rs") {
            info.project_type = "library".into();
        }
        if has_dep(root, "Cargo.toml", "actix-web") {
            info.frameworks.push("actix-web".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "Cargo.toml", "axum") {
            info.frameworks.push("axum".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "Cargo.toml", "rocket") {
            info.frameworks.push("rocket".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "Cargo.toml", "warp") {
            info.frameworks.push("warp".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "Cargo.toml", "tauri") {
            info.frameworks.push("tauri".into());
            info.project_type = "desktop-app".into();
        }
        if has_dep(root, "Cargo.toml", "leptos") {
            info.frameworks.push("leptos".into());
            info.project_type = "web-app".into();
        }
        if has_dep(root, "Cargo.toml", "yew") {
            info.frameworks.push("yew".into());
            info.project_type = "web-app".into();
        }
        if has_dep(root, "Cargo.toml", "diesel") {
            info.frameworks.push("diesel".into());
        }
        if has_dep(root, "Cargo.toml", "sqlx") {
            info.frameworks.push("sqlx".into());
        }
        if has_dep(root, "Cargo.toml", "tokio") {
            info.frameworks.push("tokio".into());
        }
        if has_dep(root, "Cargo.toml", "clap") {
            info.frameworks.push("clap-cli".into());
        }
    }
    // ── Python ────────────────────────────────────────────
    else if has_any_file(root, &["pyproject.toml", "setup.py", "setup.cfg", "requirements.txt", "Pipfile"]) {
        info.language = "python".into();

        if has_file(root, "pyproject.toml") {
            info.marker_files.push("pyproject.toml".into());
            if has_dep(root, "pyproject.toml", "poetry") {
                info.package_manager = "poetry".into();
                info.build_tool = "poetry".into();
            }
        }
        if has_file(root, "requirements.txt") {
            info.marker_files.push("requirements.txt".into());
            info.package_manager = "pip".into();
        }
        if has_file(root, "Pipfile") {
            info.package_manager = "pipenv".into();
        }

        // Python 框架检测
        if has_dep_any(root, &["requirements.txt", "pyproject.toml"], "django") {
            info.frameworks.push("django".into());
            info.project_type = "web-app".into();
            info.run_command = "python manage.py runserver".into();
            info.test_command = "python manage.py test".into();
        }
        if has_dep_any(root, &["requirements.txt", "pyproject.toml"], "flask") {
            info.frameworks.push("flask".into());
            info.project_type = "api".into();
            info.run_command = "flask run".into();
            info.test_command = "pytest".into();
        }
        if has_dep_any(root, &["requirements.txt", "pyproject.toml"], "fastapi") {
            info.frameworks.push("fastapi".into());
            info.project_type = "api".into();
            info.run_command = "uvicorn main:app --reload".into();
            info.test_command = "pytest".into();
        }
        if has_dep_any(root, &["requirements.txt", "pyproject.toml"], "torch") {
            info.frameworks.push("pytorch".into());
            info.project_type = "ml".into();
        }
        if has_dep_any(root, &["requirements.txt", "pyproject.toml"], "tensorflow") {
            info.frameworks.push("tensorflow".into());
            info.project_type = "ml".into();
        }
        if has_dep_any(root, &["requirements.txt", "pyproject.toml"], "streamlit") {
            info.frameworks.push("streamlit".into());
            info.project_type = "web-app".into();
            info.run_command = "streamlit run app.py".into();
        }

        if info.run_command == "unknown" {
            info.run_command = "python main.py".into();
            info.test_command = "pytest".into();
            info.build_command = "pip install -r requirements.txt".into();
        }
    }
    // ── JavaScript/TypeScript ─────────────────────────────
    else if has_any_file(root, &["package.json", "tsconfig.json"]) {
        info.language = "javascript".into();
        info.marker_files.push("package.json".into());

        // 包管理器检测
        if has_file(root, "pnpm-lock.yaml") {
            info.package_manager = "pnpm".into();
        } else if has_file(root, "yarn.lock") {
            info.package_manager = "yarn".into();
        } else if has_file(root, "bun.lockb") {
            info.package_manager = "bun".into();
        } else {
            info.package_manager = "npm".into();
        }

        if has_file(root, "tsconfig.json") {
            info.language = "typescript".into();
            info.marker_files.push("tsconfig.json".into());
        }

        let pm = &info.package_manager;
        info.run_command = format!("{} run dev", pm);
        info.test_command = format!("{} test", pm);
        info.build_command = format!("{} run build", pm);

        // JS/TS 框架检测
        if has_file(root, "next.config.js") || has_file(root, "next.config.ts") || has_file(root, "next.config.mjs") {
            info.frameworks.push("nextjs".into());
            info.project_type = "web-app".into();
            if has_dir(root, "src/app") || has_dir(root, "app") {
                info.frameworks.push("nextjs-app-router".into());
            } else {
                info.frameworks.push("nextjs-pages-router".into());
            }
        }
        if has_file(root, "nuxt.config.js") || has_file(root, "nuxt.config.ts") {
            info.frameworks.push("nuxt".into());
            info.project_type = "web-app".into();
        }
        if has_file(root, "vite.config.js") || has_file(root, "vite.config.ts") {
            info.frameworks.push("vite".into());
        }
        if has_dep(root, "package.json", "react") {
            info.frameworks.push("react".into());
        }
        if has_dep(root, "package.json", "vue") {
            info.frameworks.push("vue".into());
        }
        if has_dep(root, "package.json", "svelte") {
            info.frameworks.push("svelte".into());
        }
        if has_dep(root, "package.json", "express") {
            info.frameworks.push("express".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "package.json", "fastify") {
            info.frameworks.push("fastify".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "package.json", "nestjs") {
            info.frameworks.push("nestjs".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "package.json", "electron") {
            info.frameworks.push("electron".into());
            info.project_type = "desktop-app".into();
        }
        if has_dep(root, "package.json", "react-native") {
            info.frameworks.push("react-native".into());
            info.project_type = "mobile-app".into();
        }
        if has_dep(root, "package.json", "expo") {
            info.frameworks.push("expo".into());
            info.project_type = "mobile-app".into();
        }
    }
    // ── Go ────────────────────────────────────────────────
    else if has_file(root, "go.mod") {
        info.language = "go".into();
        info.build_tool = "go".into();
        info.package_manager = "go".into();
        info.run_command = "go run .".into();
        info.test_command = "go test ./...".into();
        info.build_command = "go build".into();
        info.marker_files.push("go.mod".into());

        if has_dep(root, "go.mod", "gin-gonic/gin") {
            info.frameworks.push("gin".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "go.mod", "labstack/echo") {
            info.frameworks.push("echo".into());
            info.project_type = "api".into();
        }
        if has_dep(root, "go.mod", "gofiber/fiber") {
            info.frameworks.push("fiber".into());
            info.project_type = "api".into();
        }
    }
    // ── Java ──────────────────────────────────────────────
    else if has_file(root, "pom.xml") || has_file(root, "build.gradle") || has_file(root, "build.gradle.kts") {
        info.language = "java".into();

        if has_file(root, "pom.xml") {
            info.build_tool = "maven".into();
            info.package_manager = "maven".into();
            info.run_command = "mvn spring-boot:run".into();
            info.test_command = "mvn test".into();
            info.build_command = "mvn package".into();
            info.marker_files.push("pom.xml".into());
        } else {
            info.build_tool = "gradle".into();
            info.package_manager = "gradle".into();
            info.run_command = "./gradlew bootRun".into();
            info.test_command = "./gradlew test".into();
            info.build_command = "./gradlew build".into();
            info.marker_files.push("build.gradle".into());
        }

        if has_dep_any(root, &["pom.xml", "build.gradle"], "spring-boot") {
            info.frameworks.push("spring-boot".into());
            info.project_type = "api".into();
        }
    }
    // ── C/C++ ─────────────────────────────────────────────
    else if has_any_file(root, &["CMakeLists.txt", "Makefile", "meson.build"]) {
        info.language = "c".into();
        info.build_tool = "cmake".into();
        info.package_manager = "system".into();
        info.run_command = "./build/app".into();
        info.test_command = "ctest".into();
        info.build_command = "cmake --build build".into();

        if has_file(root, "CMakeLists.txt") {
            info.marker_files.push("CMakeLists.txt".into());
        }
        if has_file(root, "Makefile") {
            info.build_tool = "make".into();
            info.build_command = "make".into();
            info.marker_files.push("Makefile".into());
        }
    }

    // ── Monorepo 检测 ─────────────────────────────────────
    if has_file(root, "pnpm-workspace.yaml") || has_file(root, "lerna.json") || has_file(root, "nx.json") {
        info.project_type = "monorepo".into();
        info.frameworks.push("monorepo".into());
    }

    info
}

// ── 辅助函数 ──────────────────────────────────────────────

fn has_file(root: &Path, name: &str) -> bool {
    root.join(name).exists()
}

fn has_any_file(root: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| root.join(n).exists())
}

fn has_dir(root: &Path, name: &str) -> bool {
    root.join(name).is_dir()
}

fn has_dep(root: &Path, manifest: &str, dep: &str) -> bool {
    let path = root.join(manifest);
    if !path.exists() { return false; }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    content.contains(dep)
}

fn has_dep_any(root: &Path, manifests: &[&str], dep: &str) -> bool {
    manifests.iter().any(|m| has_dep(root, m, dep))
}
