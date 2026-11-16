//! 清理模型给的路径：只留相对于工作目录的普通路径段。
//!
//! 真正的边界检查在打开时做（`safe_fs`），和打开是同一步；这里只挡掉一眼就能看出不对的写法。

use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    #[error("路径必须相对于工作目录：{0}")]
    Absolute(String),
    #[error("路径不能包含 `..`：{0}")]
    ParentDir(String),
    #[error("路径指向工作目录之外：{0}")]
    Escapes(String),
    #[error("路径经过不能安全跟随的符号链接：{0}")]
    Symlink(String),
    #[error("无法解析路径 {0}：{1}")]
    Io(String, String),
}

/// 返回清理后的相对路径（去掉 `.`），空表示工作目录本身。拒绝绝对路径和 `..`。
pub fn relative(raw: &str) -> Result<PathBuf, PathError> {
    let mut clean = PathBuf::new();
    for c in Path::new(raw).components() {
        match c {
            Component::Normal(p) => clean.push(p),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathError::ParentDir(raw.into())),
            Component::RootDir | Component::Prefix(_) => return Err(PathError::Absolute(raw.into())),
        }
    }
    Ok(clean)
}
