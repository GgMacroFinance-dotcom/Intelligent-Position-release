//! 文件工具。打开和读写都是同步系统调用（见 `safe_fs`），放在 `spawn_blocking` 里执行，不阻塞运行时。

use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use serde::Deserialize;

use crate::paths::PathError;
use crate::safe_fs::{self, Root};
use crate::{ToolOutput, Toolbox, paths, truncate_output};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteArgs {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

/// 最多列出的目录项数。
const MAX_ENTRIES: usize = 1000;

/// 新建文件的权限，和 `std::fs::write` 一样，再由 umask 收紧。
const FILE_MODE: u32 = 0o666;

/// 在阻塞线程里打开工作目录，再执行 `f`。`f` 出错时返回给模型的说明。
async fn with_root(tb: &Toolbox, f: impl FnOnce(&Root) -> Result<ToolOutput, String> + Send + 'static) -> ToolOutput {
    let (workspace, openat2) = (tb.workspace().to_path_buf(), tb.openat2);
    let run = move || {
        let root = safe_fs::open_root(&workspace, openat2).map_err(|e| format!("打开工作目录失败：{}", io(e)))?;
        f(&root)
    };
    match tokio::task::spawn_blocking(run).await {
        Ok(Ok(out)) => out,
        Ok(Err(message)) => ToolOutput::err(message),
        Err(e) => ToolOutput::err(format!("文件工具内部错误：{e}")),
    }
}

fn io(e: Errno) -> std::io::Error {
    std::io::Error::from(e)
}

/// 打开失败时给模型的说明。`EXDEV` 是 `openat2` 发现路径要离开工作目录，`ELOOP` 是碰上了不能跟随的符号链接。
fn open_failed(action: &str, raw: &str, e: Errno) -> String {
    match e {
        Errno::EXDEV => PathError::Escapes(raw.into()).to_string(),
        Errno::ELOOP => PathError::Symlink(raw.into()).to_string(),
        e => format!("{action} {raw} 失败：{}", io(e)),
    }
}

fn open(root: &Root, rel: &Path, raw: &str, action: &str, flags: OFlag) -> Result<File, String> {
    // openat2 要求不建文件时 mode 为 0。
    let mode = if flags.contains(OFlag::O_CREAT) {
        Mode::from_bits_truncate(FILE_MODE)
    } else {
        Mode::empty()
    };
    safe_fs::open(root, rel, flags, mode)
        .map(File::from)
        .map_err(|e| open_failed(action, raw, e))
}

fn read_text(file: &mut File, raw: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("读取 {raw} 失败：{e}"))?;
    String::from_utf8(bytes).map_err(|_| format!("{raw} 不是 UTF-8 文本文件"))
}

fn relative(raw: &str) -> Result<PathBuf, ToolOutput> {
    paths::relative(raw).map_err(|e| ToolOutput::err(e.to_string()))
}

pub async fn read_file(tb: &Toolbox, a: ReadArgs) -> ToolOutput {
    let rel = match relative(&a.path) {
        Ok(p) => p,
        Err(out) => return out,
    };
    let max_output = tb.limits.max_output;
    with_root(tb, move |root| {
        let mut file = open(root, &rel, &a.path, "读取", OFlag::O_RDONLY)?;
        let text = read_text(&mut file, &a.path)?;
        Ok(ToolOutput::ok(truncate_output(text, max_output, 0)))
    })
    .await
}

pub async fn write_file(tb: &Toolbox, a: WriteArgs) -> ToolOutput {
    let rel = match relative(&a.path) {
        Ok(p) => p,
        Err(out) => return out,
    };
    if rel.as_os_str().is_empty() {
        return ToolOutput::err("不能写入工作目录本身");
    }
    with_root(tb, move |root| {
        if let Some(parent) = rel.parent() {
            safe_fs::create_dirs(root, parent).map_err(|e| match e {
                Errno::EXDEV | Errno::ELOOP => open_failed("创建目录", &a.path, e),
                e => format!("创建目录失败：{}", io(e)),
            })?;
        }
        // 目标若是符号链接，只有解析后仍在工作目录内才会跟随（见 `safe_fs`）。
        let flags = OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_TRUNC;
        let mut file = open(root, &rel, &a.path, "写入", flags)?;
        file.write_all(a.content.as_bytes())
            .map_err(|e| format!("写入 {} 失败：{e}", a.path))?;
        Ok(ToolOutput::ok(format!("已写入 {}（{} 字节）", a.path, a.content.len())))
    })
    .await
}

pub async fn list_dir(tb: &Toolbox, a: ListArgs) -> ToolOutput {
    let raw = a.path.unwrap_or_else(|| ".".into());
    let rel = match relative(&raw) {
        Ok(p) => p,
        Err(out) => return out,
    };
    with_root(tb, move |root| {
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY;
        let fd = safe_fs::open(root, &rel, flags, Mode::empty()).map_err(|e| open_failed("读取目录", &raw, e))?;
        let failed = |e: Errno| format!("读取目录 {raw} 失败：{}", io(e));
        let mut dir = nix::dir::Dir::from_fd(fd).map_err(failed)?;
        let mut entries = Vec::new();
        let mut more = 0usize;
        for entry in dir.iter() {
            let entry = entry.map_err(failed)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if entries.len() >= MAX_ENTRIES {
                more += 1;
                continue;
            }
            entries.push((entry.file_name().to_owned(), entry.file_type()));
        }
        let mut names: Vec<String> = entries
            .into_iter()
            .map(|(name, kind)| {
                // 有的文件系统不报类型，查一下；不跟随符号链接，和以前一样指向目录的链接不加 `/`。
                let is_dir = match kind {
                    Some(kind) => kind == nix::dir::Type::Directory,
                    None => nix::sys::stat::fstatat(&dir, name.as_c_str(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)
                        .is_ok_and(|st| st.st_mode & nix::libc::S_IFMT == nix::libc::S_IFDIR),
                };
                let mut name = name.to_string_lossy().into_owned();
                if is_dir {
                    name.push('/');
                }
                name
            })
            .collect();
        names.sort();
        let mut out = names.join("\n");
        if more > 0 {
            out.push_str(&format!("\n[还有 {more} 项未列出]"));
        }
        Ok(ToolOutput::ok(out))
    })
    .await
}

pub async fn edit_file(tb: &Toolbox, a: EditArgs) -> ToolOutput {
    if a.old_string.is_empty() {
        return ToolOutput::err("old_string 不能为空；新建文件请用 write_file");
    }
    if a.old_string == a.new_string {
        return ToolOutput::err("old_string 和 new_string 相同，没有要改的");
    }
    let rel = match relative(&a.path) {
        Ok(p) => p,
        Err(out) => return out,
    };
    with_root(tb, move |root| {
        // 读和写回用同一个文件描述符，中间路径被换掉也写不到别处。
        let mut file = open(root, &rel, &a.path, "读取", OFlag::O_RDWR)?;
        let text = read_text(&mut file, &a.path)?;
        let count = text.matches(a.old_string.as_str()).count();
        if count == 0 {
            return Err(format!(
                "在 {} 里没找到 old_string；先用 read_file 看一下当前内容",
                a.path
            ));
        }
        if count > 1 && !a.replace_all {
            return Err(format!(
                "old_string 在 {} 里出现了 {count} 次；加上前后文让它唯一，或设 replace_all",
                a.path
            ));
        }
        let updated = if a.replace_all {
            text.replace(a.old_string.as_str(), &a.new_string)
        } else {
            text.replacen(a.old_string.as_str(), &a.new_string, 1)
        };
        let write_back = |file: &mut File| -> std::io::Result<()> {
            file.rewind()?;
            file.set_len(0)?;
            file.write_all(updated.as_bytes())
        };
        write_back(&mut file).map_err(|e| format!("写入 {} 失败：{e}", a.path))?;
        Ok(ToolOutput::ok(format!("已修改 {}（替换 {count} 处）", a.path)))
    })
    .await
}
