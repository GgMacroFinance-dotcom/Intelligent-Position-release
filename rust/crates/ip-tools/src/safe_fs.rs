//! 在工作目录里打开文件和目录：边界检查和打开是同一步，不留符号链接竞态的窗口（#66）。
//!
//! 以前先把路径 canonicalize 检查一遍，再按路径打开。两步之间要是有别的进程（比如员工自己在 shell 里
//! 起的后台命令）把某一级目录换成指向外部的符号链接，打开时就会跟着链接跑到工作目录外面。文件工具
//! 在 harness 进程里执行，不在沙箱里，能读写服务账号能碰到的任何文件，所以这里是唯一的防线。
//!
//! 现在一律从工作目录的文件描述符出发：
//! - Linux 5.6 及以上用 `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)`。内核解析路径的每一步都保证
//!   不离开工作目录：工作目录里指向内部的相对符号链接照常可用，绝对路径的符号链接、跑出去的 `..`、
//!   `/proc` 里的魔法链接一律拒绝（`EXDEV` / `ELOOP`）。
//! - 其他系统，或内核、seccomp 不让用 `openat2` 时，逐级 `openat(O_NOFOLLOW)`：一次只解析一个名字，
//!   路径里任何一级是符号链接都拒绝。
//!
//! 打开之后的读、写、改都通过拿到的文件描述符进行，路径之后再被换掉也影响不到它。
//! 这里全是同步系统调用，调用方要放到 `spawn_blocking` 里。

use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{OFlag, openat};
use nix::sys::stat::{Mode, mkdirat};

/// 解析的起点：工作目录的文件描述符，以及用哪种办法往下打开。
pub(crate) struct Root {
    fd: OwnedFd,
    /// 用 `openat2(RESOLVE_BENEATH)`；否则逐级 `O_NOFOLLOW`。
    beneath: bool,
}

/// 打开工作目录本身。工作目录的路径由 harness 决定，员工改不了它的上级目录。
/// `allow_openat2` 为 false 时（只在测试里）强制逐级打开，验证不支持 `openat2` 的系统上同样安全。
pub(crate) fn open_root(workspace: &Path, allow_openat2: bool) -> Result<Root, Errno> {
    let fd = nix::fcntl::open(
        workspace,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    #[cfg(target_os = "linux")]
    let beneath = allow_openat2 && linux::openat2_supported(&fd);
    #[cfg(not(target_os = "linux"))]
    let beneath = {
        let _ = allow_openat2;
        false
    };
    Ok(Root { fd, beneath })
}

/// 打开工作目录里的 `rel`。`rel` 已由 `paths::relative` 清理过，只含普通路径段；空表示工作目录本身。
pub(crate) fn open(root: &Root, rel: &Path, flags: OFlag, mode: Mode) -> Result<OwnedFd, Errno> {
    #[cfg(target_os = "linux")]
    if root.beneath {
        return linux::beneath(&root.fd, rel, flags, mode);
    }
    walk(&root.fd, rel, flags, mode)
}

/// 建好 `rel` 这一串目录（已存在的跳过）。每一级都先用 [`open`] 安全地打开上一级，再在它的
/// 文件描述符里 `mkdirat` 一个名字，所以不会在工作目录外面建目录。
pub(crate) fn create_dirs(root: &Root, rel: &Path) -> Result<(), Errno> {
    let dir_flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY;
    let mut current = open(root, Path::new(""), dir_flags, Mode::empty())?;
    let mut prefix = PathBuf::new();
    for name in names(rel)? {
        prefix.push(name);
        current = match open(root, &prefix, dir_flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::ENOENT) => {
                match mkdirat(&current, name, Mode::from_bits_truncate(0o777)) {
                    Ok(()) | Err(Errno::EEXIST) => {}
                    Err(e) => return Err(e),
                }
                open(root, &prefix, dir_flags, Mode::empty())?
            }
            Err(e) => return Err(e),
        };
    }
    Ok(())
}

fn names(rel: &Path) -> Result<Vec<&OsStr>, Errno> {
    rel.components()
        .map(|c| match c {
            Component::Normal(name) => Ok(name),
            _ => Err(Errno::EINVAL),
        })
        .collect()
}

/// 逐级打开，每一级都不跟随符号链接。
fn walk(root: &OwnedFd, rel: &Path, flags: OFlag, mode: Mode) -> Result<OwnedFd, Errno> {
    let names = names(rel)?;
    let here = || {
        openat(
            root,
            ".",
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
    };
    let Some((last, dirs)) = names.split_last() else {
        return openat(root, ".", flags | OFlag::O_CLOEXEC, mode);
    };
    let mut dir = here()?;
    for name in dirs {
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        dir = openat(&dir, *name, flags, Mode::empty()).map_err(|e| symlink_as_eloop(&dir, name, e))?;
    }
    openat(&dir, *last, flags | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC, mode).map_err(|e| symlink_as_eloop(&dir, last, e))
}

/// `O_NOFOLLOW` 碰上符号链接时，带 `O_DIRECTORY` 报 `ENOTDIR`、不带报 `ELOOP`。统一成 `ELOOP`，好告诉模型原因。
fn symlink_as_eloop(dir: &OwnedFd, name: &OsStr, e: Errno) -> Errno {
    use nix::fcntl::AtFlags;
    let is_link = || {
        nix::sys::stat::fstatat(dir, name, AtFlags::AT_SYMLINK_NOFOLLOW)
            .is_ok_and(|st| st.st_mode & nix::libc::S_IFMT == nix::libc::S_IFLNK)
    };
    if matches!(e, Errno::ENOTDIR | Errno::ELOOP) && is_link() {
        Errno::ELOOP
    } else {
        e
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::fd::OwnedFd;
    use std::path::Path;
    use std::sync::OnceLock;

    use nix::errno::Errno;
    use nix::fcntl::{OFlag, OpenHow, ResolveFlag, openat2};
    use nix::sys::stat::Mode;

    fn how(flags: OFlag, mode: Mode) -> OpenHow {
        OpenHow::new()
            .flags(flags | OFlag::O_CLOEXEC)
            .mode(mode)
            .resolve(ResolveFlag::RESOLVE_BENEATH | ResolveFlag::RESOLVE_NO_MAGICLINKS)
    }

    pub(super) fn beneath(root: &OwnedFd, rel: &Path, flags: OFlag, mode: Mode) -> Result<OwnedFd, Errno> {
        let path = if rel.as_os_str().is_empty() {
            Path::new(".")
        } else {
            rel
        };
        // 解析途中碰上并发改名时内核可能返回 EAGAIN，要调用方重试。
        let mut retries = 0;
        loop {
            match openat2(root, path, how(flags, mode)) {
                Err(Errno::EAGAIN) if retries < 8 => retries += 1,
                result => return result,
            }
        }
    }

    /// 第一次用时探测一次：内核太老（ENOSYS）或 seccomp 拦了（EPERM）就改用逐级打开。
    pub(super) fn openat2_supported(root: &OwnedFd) -> bool {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        *SUPPORTED.get_or_init(
            || match openat2(root, ".", how(OFlag::O_RDONLY | OFlag::O_DIRECTORY, Mode::empty())) {
                Err(e @ (Errno::ENOSYS | Errno::EPERM)) => {
                    tracing::warn!(error = %e, "用不了 openat2，文件工具改为不跟随任何符号链接");
                    false
                }
                _ => true,
            },
        )
    }
}
