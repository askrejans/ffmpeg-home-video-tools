//! Locating and running the `ffmpeg` and `ffprobe` executables.

use crate::error::{Error, Result};
use crate::events::CancelToken;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

/// Optional explicit locations of the FFmpeg executables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPaths {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
}

/// How input files are handed to FFmpeg child processes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputAccess {
    /// Pass file paths on the command line.
    #[default]
    Path,
    /// Open each input in this process and pass it to the child as an
    /// inherited file descriptor. Useful for sandboxed hosts whose child
    /// processes cannot open the user's files themselves. Unix only; behaves
    /// like `Path` elsewhere.
    InheritedFd,
}

/// The FFmpeg executables used by every job.
#[derive(Debug, Clone)]
pub struct FfmpegTools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    access: InputAccess,
    filters: Arc<OnceLock<BTreeSet<String>>>,
    encoders: Arc<OnceLock<BTreeSet<String>>>,
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn find_tool(
    name: &'static str,
    explicit: Option<&Path>,
    env_var: &str,
    sibling_dir: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            Err(Error::ToolNotFound { tool: name })
        };
    }
    if let Some(dir) = sibling_dir {
        let candidate = dir.join(exe_name(name));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Some(path) = std::env::var_os(env_var).map(PathBuf::from)
        && path.is_file()
    {
        return Ok(path);
    }
    which::which(name).map_err(|_| Error::ToolNotFound { tool: name })
}

impl FfmpegTools {
    /// Find `ffmpeg` and `ffprobe`: explicit paths first, then `ffprobe` next
    /// to an explicit `ffmpeg`, then the `FFMPEG_PATH`/`FFPROBE_PATH`
    /// environment variables, then `PATH`.
    pub fn locate(paths: &ToolPaths) -> Result<Self> {
        let ffmpeg = find_tool("ffmpeg", paths.ffmpeg.as_deref(), "FFMPEG_PATH", None)?;
        let sibling = paths.ffmpeg.as_ref().and_then(|_| ffmpeg.parent());
        let ffprobe = find_tool("ffprobe", paths.ffprobe.as_deref(), "FFPROBE_PATH", sibling)?;
        Ok(Self {
            ffmpeg,
            ffprobe,
            access: InputAccess::Path,
            filters: Arc::default(),
            encoders: Arc::default(),
        })
    }

    pub fn with_input_access(mut self, access: InputAccess) -> Self {
        self.access = access;
        self
    }

    pub fn input_access(&self) -> InputAccess {
        self.access
    }

    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg
    }

    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe
    }

    /// First line of `ffmpeg -version`.
    pub fn version(&self) -> Result<String> {
        let mut cmd = self.ffmpeg();
        cmd.arg("-version");
        let out = cmd.output(&CancelToken::new())?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string())
    }

    /// Names of the filters compiled into this FFmpeg build.
    pub fn filters(&self) -> Result<&BTreeSet<String>> {
        if let Some(set) = self.filters.get() {
            return Ok(set);
        }
        let mut cmd = self.ffmpeg();
        cmd.arg("-filters");
        let out = cmd.output(&CancelToken::new())?;
        let set = parse_name_list(&out, 1);
        Ok(self.filters.get_or_init(|| set))
    }

    /// Names of the encoders compiled into this FFmpeg build.
    pub fn encoders(&self) -> Result<&BTreeSet<String>> {
        if let Some(set) = self.encoders.get() {
            return Ok(set);
        }
        let mut cmd = self.ffmpeg();
        cmd.arg("-encoders");
        let out = cmd.output(&CancelToken::new())?;
        let set = parse_name_list(&out, 1);
        Ok(self.encoders.get_or_init(|| set))
    }

    pub fn has_filter(&self, name: &str) -> bool {
        self.filters().map(|f| f.contains(name)).unwrap_or(false)
    }

    pub(crate) fn ffmpeg(&self) -> ToolCommand {
        let mut cmd = ToolCommand::new("ffmpeg", &self.ffmpeg, self.access);
        cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
        cmd
    }

    pub(crate) fn ffprobe(&self) -> ToolCommand {
        let mut cmd = ToolCommand::new("ffprobe", &self.ffprobe, self.access);
        cmd.args(["-hide_banner", "-v", "error"]);
        cmd
    }
}

/// Parse `ffmpeg -filters` / `-encoders` output: a header, a `---` or
/// `------` separator line, then one entry per line with the name in the
/// given whitespace-separated column.
fn parse_name_list(out: &[u8], column: usize) -> BTreeSet<String> {
    let text = String::from_utf8_lossy(out);
    let mut started = false;
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !started {
            if trimmed.starts_with("---") || trimmed == "------" {
                started = true;
            }
            continue;
        }
        if let Some(name) = trimmed.split_whitespace().nth(column) {
            names.insert(name.to_string());
        }
    }
    names
}

/// `file:` URL for a path, so names containing `:` are never mistaken for a
/// protocol.
pub(crate) fn file_url(path: &Path) -> OsString {
    let mut url = OsString::from("file:");
    url.push(path.as_os_str());
    url
}

/// A command line for one FFmpeg tool, with input handling.
pub(crate) struct ToolCommand {
    tool: &'static str,
    cmd: Command,
    access: InputAccess,
    #[cfg(unix)]
    fds: Vec<command_fds::FdMapping>,
    #[cfg(unix)]
    next_fd: i32,
}

impl ToolCommand {
    fn new(tool: &'static str, path: &Path, access: InputAccess) -> Self {
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        Self {
            tool,
            cmd,
            access,
            #[cfg(unix)]
            fds: Vec::new(),
            #[cfg(unix)]
            next_fd: 3,
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.cmd.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd.args(args);
        self
    }

    /// Register an input file and return the URL to use for it. With
    /// [`InputAccess::InheritedFd`] this opens the file here and adds the
    /// `-fd N` option, so it must be called right before the URL is added.
    pub fn input_url(&mut self, path: &Path) -> Result<OsString> {
        #[cfg(unix)]
        if self.access == InputAccess::InheritedFd {
            use std::os::fd::OwnedFd;
            let file = std::fs::File::open(path).map_err(|e| Error::Unreadable {
                path: path.to_path_buf(),
                reason: e.to_string(),
            })?;
            let child_fd = self.next_fd;
            self.next_fd += 1;
            self.fds.push(command_fds::FdMapping {
                parent_fd: OwnedFd::from(file),
                child_fd,
            });
            self.cmd.arg("-fd").arg(child_fd.to_string());
            return Ok(OsString::from("fd:"));
        }
        Ok(file_url(path))
    }

    /// Add `-i <input>` for `path`.
    pub fn input(&mut self, path: &Path) -> Result<&mut Self> {
        let url = self.input_url(path)?;
        self.cmd.arg("-i").arg(url);
        Ok(self)
    }

    /// Add `-i file:<path>` for a file the job created itself (always passed
    /// by path, whatever the input access mode).
    pub fn input_path(&mut self, path: &Path) -> &mut Self {
        self.cmd.arg("-i").arg(file_url(path));
        self
    }

    /// Add an output file argument.
    pub fn output_path(&mut self, path: &Path) -> &mut Self {
        self.cmd.arg(file_url(path));
        self
    }

    fn prepare(&mut self) -> Result<()> {
        #[cfg(unix)]
        if !self.fds.is_empty() {
            use command_fds::CommandFdExt;
            let fds = std::mem::take(&mut self.fds);
            self.cmd
                .fd_mappings(fds)
                .map_err(|e| Error::io("passing input descriptors", std::io::Error::other(e)))?;
        }
        Ok(())
    }

    /// Start the process with piped stderr (collected for error messages).
    pub fn spawn(mut self, stdin: Stdio, stdout: Stdio) -> Result<RunningTool> {
        self.prepare()?;
        tracing::debug!(tool = self.tool, command = ?self.cmd, "spawning");
        self.cmd.stdin(stdin).stdout(stdout).stderr(Stdio::piped());
        let mut child = match self.cmd.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::ToolNotFound { tool: self.tool });
            }
            Err(e) => return Err(Error::io(format!("starting {}", self.tool), e)),
        };
        let stderr = Arc::new(Mutex::new(StderrTail::default()));
        let stderr_thread = child.stderr.take().map(|mut pipe| {
            let tail = Arc::clone(&stderr);
            let tool = self.tool;
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let chunk = String::from_utf8_lossy(&buf[..n]);
                    tracing::trace!(tool, "{}", chunk.trim_end());
                    if let Ok(mut tail) = tail.lock() {
                        tail.push(&chunk);
                    }
                }
            })
        });
        Ok(RunningTool {
            tool: self.tool,
            child,
            stderr,
            stderr_thread,
            finished: false,
        })
    }

    /// Run to completion and return stdout. Fails on a non-zero exit.
    pub fn output(self, cancel: &CancelToken) -> Result<Vec<u8>> {
        let mut running = self.spawn(Stdio::null(), Stdio::piped())?;
        let mut stdout = running.child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            out
        });
        running.wait(cancel)?;
        Ok(reader.join().unwrap_or_default())
    }

    /// Run to completion and return stdout plus everything written to stderr
    /// (up to the last 8 KiB).
    pub fn output_with_stderr(self, cancel: &CancelToken) -> Result<(Vec<u8>, String)> {
        let mut running = self.spawn(Stdio::null(), Stdio::piped())?;
        let mut stdout = running.child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            out
        });
        running.wait(cancel)?;
        let stderr = running.stderr_text();
        Ok((reader.join().unwrap_or_default(), stderr))
    }

    /// Run to completion, discarding stdout.
    pub fn run(self, cancel: &CancelToken) -> Result<()> {
        let mut running = self.spawn(Stdio::null(), Stdio::null())?;
        running.wait(cancel)
    }
}

#[derive(Default)]
struct StderrTail {
    text: String,
}

impl StderrTail {
    const LIMIT: usize = 8 * 1024;

    fn push(&mut self, chunk: &str) {
        self.text.push_str(chunk);
        if self.text.len() > Self::LIMIT {
            let mut cut = self.text.len() - Self::LIMIT;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
    }
}

/// A running FFmpeg process. Dropping it kills the process.
pub(crate) struct RunningTool {
    tool: &'static str,
    child: Child,
    stderr: Arc<Mutex<StderrTail>>,
    stderr_thread: Option<JoinHandle<()>>,
    finished: bool,
}

impl RunningTool {
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn kill(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.finished = true;
        }
    }

    pub fn stderr_text(&mut self) -> String {
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        self.stderr
            .lock()
            .map(|t| t.text.trim().to_string())
            .unwrap_or_default()
    }

    /// Wait for exit, polling `cancel`. A non-zero exit becomes
    /// [`Error::ToolFailed`] carrying the tail of stderr.
    pub fn wait(&mut self, cancel: &CancelToken) -> Result<()> {
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => return Err(Error::io(format!("waiting for {}", self.tool), e)),
            }
            if cancel.is_cancelled() {
                self.kill();
                return Err(Error::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(15));
        };
        self.finished = true;
        self.check_status(status)
    }

    fn check_status(&mut self, status: ExitStatus) -> Result<()> {
        if status.success() {
            return Ok(());
        }
        let stderr = self.stderr_text();
        Err(Error::ToolFailed {
            tool: self.tool,
            status: status.code(),
            stderr: if stderr.is_empty() {
                "no error output".to_string()
            } else {
                stderr
            },
        })
    }
}

impl Drop for RunningTool {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filter_list() {
        let out = b"Filters:\n  T.. = Timeline support\n  ------\n ... abench            A->A       Benchmark part of a filtergraph.\n TSC gblur             V->V       Apply Gaussian Blur filter.\n";
        let names = parse_name_list(out, 1);
        assert!(names.contains("gblur"));
        assert!(names.contains("abench"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn parses_encoder_list() {
        let out = b"Encoders:\n V..... = Video\n ------\n V....D libx264              libx264 H.264\n A....D aac                  AAC (Advanced Audio Coding)\n";
        let names = parse_name_list(out, 1);
        assert!(names.contains("libx264"));
        assert!(names.contains("aac"));
    }

    #[test]
    fn stderr_tail_keeps_the_end() {
        let mut tail = StderrTail::default();
        tail.push(&"a".repeat(StderrTail::LIMIT));
        tail.push("ā-end");
        assert!(tail.text.ends_with("ā-end"));
        assert!(tail.text.len() <= StderrTail::LIMIT + 1);
    }

    #[test]
    fn explicit_missing_tool_is_reported() {
        let err = FfmpegTools::locate(&ToolPaths {
            ffmpeg: Some(PathBuf::from("/definitely/not/here/ffmpeg")),
            ffprobe: None,
        })
        .unwrap_err();
        assert_eq!(err.code(), "tool_not_found");
    }
}
