//! 看员工写的报告（backlog F-4）：列出、读取员工工作目录 `reports/` 下的 Markdown 文件。
//!
//! 只读，只认 `reports/` 这一层里的 `.md` 文件：路径里不能有子目录、`..`、绝对路径和隐藏文件，
//! 符号链接一律不认（列表里跳过，读取时拒绝），解开后的真实路径还必须在这位员工的 `reports/` 里。
//! 员工没有工作目录（`tools: false`）时没有报告。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ip_core::AgentId;

use crate::App;
use crate::protocol::{ReportInfo, ReportText};

/// 报告所在的子目录，岗位说明里也是这么写的。
pub const REPORTS_DIR: &str = "reports";
/// 一次最多列多少份。
pub const MAX_LIST: usize = 500;
/// 一份报告最多读多少字节，超过的截断并标记。
pub const MAX_READ_BYTES: u64 = 1 << 20;

pub async fn list(app: &App, agent: Option<&AgentId>, limit: usize) -> Result<Vec<ReportInfo>, String> {
    let workspaces = app.workspaces.clone();
    let agent = agent.cloned();
    tokio::task::spawn_blocking(move || list_in(&workspaces, agent.as_ref(), limit))
        .await
        .map_err(|e| e.to_string())?
}

pub async fn read(app: &App, agent: &AgentId, path: &str) -> Result<ReportText, String> {
    let workspaces = app.workspaces.clone();
    let agent = agent.clone();
    let path = path.to_string();
    tokio::task::spawn_blocking(move || read_in(&workspaces, &agent, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// 列出报告，新改的在前。`agent` 为空时列所有有工作目录的员工。
pub fn list_in(
    workspaces: &HashMap<AgentId, PathBuf>,
    agent: Option<&AgentId>,
    limit: usize,
) -> Result<Vec<ReportInfo>, String> {
    let targets: Vec<(&AgentId, &PathBuf)> = match agent {
        Some(id) => vec![(id, workspace(workspaces, id)?)],
        None => workspaces.iter().collect(),
    };
    let mut out = Vec::new();
    for (id, ws) in targets {
        let dir = ws.join(REPORTS_DIR);
        // reports 本身是符号链接时不列，和读取时的规则一致。
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("读不了 {} 的报告目录：{e}", id)),
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if valid_name(name).is_err() {
                continue;
            }
            // DirEntry::metadata 不跟随符号链接，所以符号链接在这里就被跳过了。
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            out.push(ReportInfo {
                agent_id: id.clone(),
                path: format!("{REPORTS_DIR}/{name}"),
                bytes: meta.len(),
                modified_ms: modified_ms(&meta),
            });
        }
    }
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then_with(|| b.path.cmp(&a.path)));
    out.truncate(limit.min(MAX_LIST));
    Ok(out)
}

/// 读一份报告。`path` 是 `list` 给出的 `reports/<文件名>.md`。
pub fn read_in(workspaces: &HashMap<AgentId, PathBuf>, agent: &AgentId, path: &str) -> Result<ReportText, String> {
    let ws = workspace(workspaces, agent)?;
    let name = path
        .strip_prefix(REPORTS_DIR)
        .and_then(|p| p.strip_prefix('/'))
        .ok_or_else(|| format!("只能读 {REPORTS_DIR}/ 下的报告"))?;
    valid_name(name)?;
    let dir = ws.join(REPORTS_DIR);
    let file = dir.join(name);
    let not_found = || format!("{} 没有报告 {path}", agent);
    let meta = std::fs::symlink_metadata(&file).map_err(|_| not_found())?;
    if !meta.file_type().is_file() {
        return Err(not_found());
    }
    // 再核对一次真实路径，防止 reports 目录本身被换成指向别处的符号链接。
    let real_dir = dir.canonicalize().map_err(|_| not_found())?;
    let real_ws = ws.canonicalize().map_err(|_| not_found())?;
    let real = file.canonicalize().map_err(|_| not_found())?;
    if real.parent() != Some(real_dir.as_path()) || !real_dir.starts_with(&real_ws) {
        return Err(not_found());
    }
    let f = open_regular(&real, &meta).map_err(|e| match e {
        OpenError::Changed => not_found(),
        OpenError::Io(e) => format!("读报告 {path} 失败：{e}"),
    })?;
    let mut bytes = Vec::new();
    f.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读报告 {path} 失败：{e}"))?;
    let truncated = bytes.len() as u64 > MAX_READ_BYTES;
    if truncated {
        bytes.truncate(MAX_READ_BYTES as usize);
    }
    Ok(ReportText {
        agent_id: agent.clone(),
        path: format!("{REPORTS_DIR}/{name}"),
        text: String::from_utf8_lossy(&bytes).into_owned(),
        modified_ms: modified_ms(&meta),
        truncated,
    })
}

enum OpenError {
    /// 检查之后、打开之前文件被换掉了（换成符号链接、管道或别的文件）。
    Changed,
    Io(std::io::Error),
}

/// 打开前面检查过的那个普通文件。员工能在自己的工作目录里跑命令，可能在检查和打开之间把文件换掉，
/// 所以不跟随符号链接、不在管道上卡住，打开后再核对一次是不是同一个文件。
#[cfg(unix)]
fn open_regular(path: &Path, checked: &std::fs::Metadata) -> Result<std::fs::File, OpenError> {
    use nix::fcntl::OFlag;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(code) if code == nix::errno::Errno::ELOOP as i32 => OpenError::Changed,
            _ => OpenError::Io(e),
        })?;
    let opened = f.metadata().map_err(OpenError::Io)?;
    if !opened.is_file() || (opened.dev(), opened.ino()) != (checked.dev(), checked.ino()) {
        return Err(OpenError::Changed);
    }
    Ok(f)
}

#[cfg(not(unix))]
fn open_regular(path: &Path, _checked: &std::fs::Metadata) -> Result<std::fs::File, OpenError> {
    std::fs::File::open(path).map_err(OpenError::Io)
}

fn workspace<'a>(workspaces: &'a HashMap<AgentId, PathBuf>, agent: &AgentId) -> Result<&'a PathBuf, String> {
    workspaces
        .get(agent)
        .ok_or_else(|| format!("{agent} 没有工作目录，没有报告"))
}

/// 文件名只能是一层、不以点开头、以 `.md` 结尾。
fn valid_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if ok {
        Ok(())
    } else {
        Err(format!("{name:?} 不是报告文件名（只认 {REPORTS_DIR}/ 下的 .md 文件）"))
    }
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, HashMap<AgentId, PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(ws.join(REPORTS_DIR).join("sub")).unwrap();
        std::fs::write(ws.join(REPORTS_DIR).join("2026-11-15-ppi.md"), "# PPI").unwrap();
        std::fs::write(ws.join(REPORTS_DIR).join("2026-11-16-cpi.md"), "# CPI\n\n| a | b |").unwrap();
        std::fs::write(ws.join(REPORTS_DIR).join("notes.txt"), "不是报告").unwrap();
        std::fs::write(ws.join(REPORTS_DIR).join(".hidden.md"), "隐藏").unwrap();
        std::fs::write(ws.join(REPORTS_DIR).join("sub").join("deep.md"), "子目录").unwrap();
        std::fs::write(dir.path().join("secret.md"), "工作目录外").unwrap();
        // 让两份报告的修改时间有先后。
        let older = std::fs::File::options()
            .write(true)
            .open(ws.join(REPORTS_DIR).join("2026-11-15-ppi.md"))
            .unwrap();
        older
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        let workspaces = HashMap::from([(AgentId::from("macro-1"), ws)]);
        (dir, workspaces)
    }

    #[test]
    fn lists_markdown_reports_newest_first() {
        let (_dir, ws) = setup();
        let list = list_in(&ws, Some(&"macro-1".into()), 10).unwrap();
        let paths: Vec<&str> = list.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["reports/2026-11-16-cpi.md", "reports/2026-11-15-ppi.md"]);
        assert_eq!(list[0].bytes, "# CPI\n\n| a | b |".len() as u64);
        // 不指定员工时列所有员工的
        assert_eq!(list_in(&ws, None, 10).unwrap().len(), 2);
        assert_eq!(list_in(&ws, None, 1).unwrap().len(), 1);
    }

    #[test]
    fn missing_reports_dir_is_empty_and_unknown_agent_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let ws = HashMap::from([(AgentId::from("ops-1"), dir.path().to_path_buf())]);
        assert!(list_in(&ws, Some(&"ops-1".into()), 10).unwrap().is_empty());
        assert!(list_in(&ws, Some(&"nobody".into()), 10).is_err());
        assert!(read_in(&ws, &"nobody".into(), "reports/a.md").is_err());
    }

    #[test]
    fn reads_a_report() {
        let (_dir, ws) = setup();
        let r = read_in(&ws, &"macro-1".into(), "reports/2026-11-16-cpi.md").unwrap();
        assert_eq!(r.text, "# CPI\n\n| a | b |");
        assert!(!r.truncated);
        assert!(r.modified_ms > 0);
    }

    #[test]
    fn refuses_paths_outside_reports() {
        let (_dir, ws) = setup();
        let id: AgentId = "macro-1".into();
        for bad in [
            "../secret.md",
            "reports/../../secret.md",
            "reports/../reports/2026-11-16-cpi.md",
            "/etc/passwd",
            "reports//etc/passwd.md",
            "reports/sub/deep.md",
            "reports/.hidden.md",
            "reports/notes.txt",
            "reports/",
            "2026-11-16-cpi.md",
            "reports\\2026-11-16-cpi.md",
            "reports/missing.md",
        ] {
            assert!(read_in(&ws, &id, bad).is_err(), "{bad} 应该被拒绝");
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks() {
        let (dir, ws) = setup();
        let reports = ws[&AgentId::from("macro-1")].join(REPORTS_DIR);
        std::os::unix::fs::symlink(dir.path().join("secret.md"), reports.join("link.md")).unwrap();
        assert!(read_in(&ws, &"macro-1".into(), "reports/link.md").is_err());
        let list = list_in(&ws, Some(&"macro-1".into()), 10).unwrap();
        assert!(list.iter().all(|r| r.path != "reports/link.md"));

        // reports 目录本身是指向工作目录外的符号链接时也不认。
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(other.path().join("outside")).unwrap();
        std::fs::write(other.path().join("outside").join("x.md"), "外面").unwrap();
        let ws2 = other.path().join("ws2");
        std::fs::create_dir_all(&ws2).unwrap();
        std::os::unix::fs::symlink(other.path().join("outside"), ws2.join(REPORTS_DIR)).unwrap();
        let map = HashMap::from([(AgentId::from("a"), ws2)]);
        assert!(read_in(&map, &"a".into(), "reports/x.md").is_err());
        assert!(list_in(&map, Some(&"a".into()), 10).unwrap().is_empty());
    }

    /// 检查之后文件被换掉（换成别的文件、符号链接、管道）时不读，也不会卡在管道上。
    #[cfg(unix)]
    #[test]
    fn refuses_files_swapped_after_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.md");
        std::fs::write(&path, "原来的").unwrap();
        let checked = std::fs::symlink_metadata(&path).unwrap();
        assert!(open_regular(&path, &checked).is_ok());

        let other = dir.path().join("other.md");
        std::fs::write(&other, "换进来的").unwrap();
        std::fs::rename(&other, &path).unwrap();
        assert!(matches!(open_regular(&path, &checked), Err(OpenError::Changed)));

        std::fs::remove_file(&path).unwrap();
        std::fs::write(dir.path().join("secret"), "外面").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret"), &path).unwrap();
        assert!(matches!(open_regular(&path, &checked), Err(OpenError::Changed)));

        std::fs::remove_file(&path).unwrap();
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::S_IRWXU).unwrap();
        assert!(matches!(open_regular(&path, &checked), Err(OpenError::Changed)));
    }

    #[test]
    fn truncates_large_reports() {
        let (_dir, ws) = setup();
        let big = "数".repeat(MAX_READ_BYTES as usize); // 每个字 3 字节，远超上限
        std::fs::write(ws[&AgentId::from("macro-1")].join(REPORTS_DIR).join("big.md"), &big).unwrap();
        let r = read_in(&ws, &"macro-1".into(), "reports/big.md").unwrap();
        assert!(r.truncated);
        assert!(r.text.len() <= MAX_READ_BYTES as usize + 3);
    }
}
