//! 内置工具与沙箱：`shell`、`read_file`、`write_file`、`list_dir`、`edit_file`。
//!
//! 每个数字员工有一个自己的工作目录（workspace）。`shell` 在 bubblewrap 沙箱里执行：
//! 系统目录只读、工作目录可写、默认断网、环境变量全部清空（API key 永远不会进到工具进程里）。
//! 文件工具在本进程里执行，从工作目录的文件描述符出发打开文件，边界检查和打开是同一步（见 `safe_fs`），
//! 拒绝绝对路径、`..` 和指向外部的符号链接，也不怕检查之后路径被换掉。
//!
//! 设计参考 codex 的 linux-sandbox（Apache-2.0），没有复制代码。

mod fs;
mod paths;
mod safe_fs;
mod sandbox;
mod shell;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::Duration;

use ip_models::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use paths::PathError;
pub use sandbox::Sandbox;

/// 单次工具调用的资源上限。
#[derive(Clone, Debug)]
pub struct Limits {
    /// `shell` 默认超时；模型可以在参数里要求更短或更长，但不超过 `max_timeout`。
    pub timeout: Duration,
    pub max_timeout: Duration,
    /// 返回给模型的输出上限（字节）。超出部分丢弃并注明省略了多少，避免一次 `cat` 撑爆上下文。
    pub max_output: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            max_timeout: Duration::from_secs(600),
            max_output: 64 * 1024,
        }
    }
}

/// 工具执行结果，原样作为 tool 消息回给模型。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub ok: bool,
    pub output: String,
}

impl ToolOutput {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
        }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self {
            ok: false,
            output: output.into(),
        }
    }
}

/// 一个数字员工可用的内置工具集合。克隆开销很小，可以在任务之间共享。
#[derive(Clone, Debug)]
pub struct Toolbox {
    workspace: PathBuf,
    sandbox: Sandbox,
    limits: Limits,
    /// 文件工具能不能用 `openat2`。只有测试会关掉它，验证逐级打开那条路。
    openat2: bool,
}

impl Toolbox {
    /// `workspace` 必须已存在；内部保存它的规范化路径，后续路径校验都以此为准。
    pub fn new(workspace: impl AsRef<Path>, sandbox: Sandbox, limits: Limits) -> std::io::Result<Self> {
        let workspace = std::fs::canonicalize(workspace)?;
        if !workspace.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "workspace 不是目录",
            ));
        }
        Ok(Self {
            workspace,
            sandbox,
            limits,
            openat2: true,
        })
    }

    /// 测试用：文件工具不用 `openat2`，改为逐级 `O_NOFOLLOW` 打开。
    #[cfg(test)]
    pub(crate) fn without_openat2(mut self) -> Self {
        self.openat2 = false;
        self
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    /// 发给模型的工具定义。
    pub fn specs(&self) -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "shell".into(),
                description: "在工作目录里用 /bin/sh 执行一条命令，返回合并后的 stdout/stderr 和退出码。没有网络。"
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "要执行的 shell 命令"},
                        "timeout_secs": {"type": "integer", "minimum": 1, "description": "超时秒数，可选"}
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "read_file".into(),
                description: "读取工作目录里的一个 UTF-8 文本文件。路径相对于工作目录。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "write_file".into(),
                description: "把内容整体写入工作目录里的文件，不存在的父目录会自动创建。路径相对于工作目录。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "list_dir".into(),
                description: "列出工作目录里某个目录的内容，目录名以 / 结尾。路径省略时为工作目录本身。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "edit_file".into(),
                description: "把文件里的一段原文替换成新内容。old_string 必须与文件内容逐字一致；\
                              默认要求它在文件里只出现一次（多处时加更多上下文，或设 replace_all）。\
                              改大文件时用它，不要整体重写。"
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "old_string": {"type": "string", "description": "要替换的原文"},
                        "new_string": {"type": "string", "description": "替换后的内容"},
                        "replace_all": {"type": "boolean", "description": "替换所有出现的地方"}
                    },
                    "required": ["path", "old_string", "new_string"],
                    "additionalProperties": false
                }),
            },
        ]
    }

    /// 执行一次工具调用。参数错误、未知工具等都以 `ok: false` 返回给模型，让它自行修正，不会 panic。
    pub async fn call(&self, name: &str, arguments: &str) -> ToolOutput {
        let args = if arguments.trim().is_empty() { "{}" } else { arguments };
        match name {
            "shell" => match serde_json::from_str::<shell::Args>(args) {
                Ok(a) => shell::run(self, a).await,
                Err(e) => bad_args(name, e),
            },
            "read_file" => match serde_json::from_str::<fs::ReadArgs>(args) {
                Ok(a) => fs::read_file(self, a).await,
                Err(e) => bad_args(name, e),
            },
            "write_file" => match serde_json::from_str::<fs::WriteArgs>(args) {
                Ok(a) => fs::write_file(self, a).await,
                Err(e) => bad_args(name, e),
            },
            "list_dir" => match serde_json::from_str::<fs::ListArgs>(args) {
                Ok(a) => fs::list_dir(self, a).await,
                Err(e) => bad_args(name, e),
            },
            "edit_file" => match serde_json::from_str::<fs::EditArgs>(args) {
                Ok(a) => fs::edit_file(self, a).await,
                Err(e) => bad_args(name, e),
            },
            other => ToolOutput::err(format!("未知工具：{other}")),
        }
    }
}

fn bad_args(tool: &str, e: serde_json::Error) -> ToolOutput {
    ToolOutput::err(format!("{tool} 的参数不合法：{e}"))
}

/// 按字节上限截断，保证不切断 UTF-8 字符，并注明省略了多少字节。
pub fn truncate_output(mut text: String, max: usize, dropped_extra: usize) -> String {
    let mut dropped = dropped_extra;
    if text.len() > max {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        dropped += text.len() - cut;
        text.truncate(cut);
    }
    if dropped > 0 {
        text.push_str(&format!("\n[输出超过上限，省略 {dropped} 字节]"));
    }
    text
}
