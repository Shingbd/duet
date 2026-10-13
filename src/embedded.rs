//! A real child terminal, rendered as cells: child escape sequences never reach the host terminal.
use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    ReviewBuffer,
    ProjectFile {
        path: String,
    },
    Project {
        path: String,
        version: String,
    },
    Review {
        task_id: String,
        path: String,
        version: String,
    },
    Conflict {
        draft: Box<crate::conflict::Draft>,
        index: usize,
    },
}
impl Selection {
    pub fn path(&self) -> &str {
        match self {
            Self::ReviewBuffer => "审核草稿",
            Self::ProjectFile { path } | Self::Project { path, .. } | Self::Review { path, .. } => {
                path
            }
            Self::Conflict { draft, index } => {
                draft.conflicts.get(*index).map_or("", String::as_str)
            }
        }
    }
}
pub struct PtyEditor {
    child: Child,
    master: File,
    parser: vt100::Parser,
    pending: Vec<u8>,
    size: (u16, u16),
    query_tail: Vec<u8>,
    exited: Option<ExitStatus>,
    stopped: bool,
}
impl PtyEditor {
    #[cfg(unix)]
    pub fn spawn(executable: &Path, cwd: &Path, path: &Path, rows: u16, cols: u16) -> Result<Self> {
        Self::spawn_managed(executable, cwd, path, rows, cols, None)
    }
    #[cfg(unix)]
    pub fn spawn_managed(
        executable: &Path,
        cwd: &Path,
        path: &Path,
        rows: u16,
        cols: u16,
        config_home: Option<&Path>,
    ) -> Result<Self> {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        };
        let (rows, cols) = (rows.clamp(1, 200), cols.clamp(1, 400));
        let mut master = -1;
        let mut slave = -1;
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // openpty returns two owned descriptors. Immediately give them RAII ownership.
        ensure!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            } == 0,
            "Cannot allocate editor PTY: {}",
            std::io::Error::last_os_error()
        );
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            ensure!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != -1,
                "Cannot protect PTY descriptor"
            );
        }
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        ensure!(
            flags != -1
                && unsafe {
                    libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
                } != -1,
            "Cannot set editor PTY nonblocking"
        );
        let mut command = Command::new(executable);
        command
            .current_dir(cwd)
            .arg(path)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        if let Some(home) = config_home {
            command
                .env("XDG_CONFIG_HOME", home)
                .env("XDG_CACHE_HOME", home.join("cache"))
                .arg("--config")
                .arg(home.join("helix/config.toml"));
        }
        // Only async-signal-safe syscalls in the post-fork child; fd 0 is already the slave.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().with_context(|| {
            format!(
                "无法启动内嵌 Helix：{}；请安装 hx 或配置 helix_path",
                executable.display()
            )
        })?;
        Ok(Self {
            child,
            master,
            parser: vt100::Parser::new(rows, cols, 500),
            pending: Vec::new(),
            size: (rows, cols),
            query_tail: Vec::new(),
            exited: None,
            stopped: false,
        })
    }
    #[cfg(not(unix))]
    pub fn spawn(
        _executable: &Path,
        _cwd: &Path,
        _path: &Path,
        _rows: u16,
        _cols: u16,
    ) -> Result<Self> {
        anyhow::bail!("Embedded Helix currently requires Unix PTY support")
    }
    #[cfg(not(unix))]
    pub fn spawn_managed(
        executable: &Path,
        cwd: &Path,
        path: &Path,
        rows: u16,
        cols: u16,
        _config_home: Option<&Path>,
    ) -> Result<Self> {
        Self::spawn(executable, cwd, path, rows, cols)
    }
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<bool> {
        let size = (rows.clamp(1, 200), cols.clamp(1, 400));
        if size == self.size {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let winsize = libc::winsize {
                ws_row: size.0,
                ws_col: size.1,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            ensure!(
                unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &winsize) } != -1,
                "Cannot resize editor PTY"
            );
        }
        self.parser.set_size(size.0, size.1);
        self.size = size;
        Ok(true)
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.pending.len() + bytes.len() <= 1024 * 1024,
            "Editor input queue is full; wait before pasting more"
        );
        self.pending.extend_from_slice(bytes);
        self.flush()
    }
    fn flush(&mut self) -> Result<()> {
        while !self.pending.is_empty() {
            match self.master.write(&self.pending) {
                Ok(0) => break,
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        let bytes = if self.screen().bracketed_paste() {
            format!("\u{1b}[200~{}\u{1b}[201~", text).into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        self.send(&bytes)
    }
    pub fn key(&mut self, event: KeyEvent) -> Result<()> {
        let bytes = encode_key(event, self.screen().application_cursor());
        self.send(&bytes)
    }
    pub fn poll(&mut self) -> Result<bool> {
        if self.exited.is_none() {
            self.exited = self.child.try_wait()?;
        }
        if self.exited.is_some() {
            self.pending.clear();
        } else {
            self.flush()?;
        }
        let mut changed = false;
        let mut buf = [0; 8192];
        for _ in 0..32 {
            match self.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.parser.process(&buf[..n]);
                    changed = true;
                    let previous = self.query_tail.len();
                    self.query_tail.extend_from_slice(&buf[..n]);
                    // Preserve incomplete probes across reads, respond once per completed sequence.
                    let tail = &self.query_tail;
                    let position = tail
                        .windows(4)
                        .enumerate()
                        .any(|(i, w)| i + 4 > previous && w == b"\x1b[6n");
                    let attributes = tail
                        .windows(3)
                        .enumerate()
                        .any(|(i, w)| i + 3 > previous && w == b"\x1b[c")
                        || tail
                            .windows(4)
                            .enumerate()
                            .any(|(i, w)| i + 4 > previous && w == b"\x1b[0c");
                    let keep = self
                        .query_tail
                        .split_off(self.query_tail.len().saturating_sub(3));
                    self.query_tail = keep;
                    if position && self.exited.is_none() {
                        let (r, c) = self.screen().cursor_position();
                        self.send(format!("\x1b[{};{}R", r + 1, c + 1).as_bytes())?;
                    }
                    if attributes && self.exited.is_none() {
                        self.send(b"\x1b[?1;2c")?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                #[cfg(unix)]
                Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                Err(e) => return Err(e.into()),
            }
        }
        if self.exited.is_none() {
            self.exited = self.child.try_wait()?;
        }
        Ok(changed)
    }
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exited
    }
    pub fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        if self.exited.is_none() {
            let _ = self.child.kill();
            self.exited = Some(self.child.wait()?);
        }
        Ok(())
    }
}
impl Drop for PtyEditor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
/// Encode legacy xterm input; Ctrl+] is reserved by the host for focus switching.
pub fn encode_key(event: KeyEvent, application_cursor: bool) -> Vec<u8> {
    if event.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let modifiers = event.modifiers;
    let control = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let mut bytes = match event.code {
        KeyCode::Char(c) if control => {
            let c = c.to_ascii_lowercase();
            if c == ' ' || c == '@' {
                vec![0]
            } else if c.is_ascii_lowercase() {
                vec![c as u8 - b'a' + 1]
            } else if ('['..='_').contains(&c) {
                vec![c as u8 - 64]
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Esc => vec![27],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End => {
            let suffix = match event.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            let mask = 1
                + u8::from(modifiers.contains(KeyModifiers::SHIFT))
                + 2 * u8::from(alt)
                + 4 * u8::from(control);
            if mask == 1 {
                format!(
                    "\x1b{}{}",
                    if application_cursor { 'O' } else { '[' },
                    suffix
                )
                .into_bytes()
            } else {
                format!("\x1b[1;{mask}{suffix}").into_bytes()
            }
        }
        KeyCode::Delete | KeyCode::Insert | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match event.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                _ => 6,
            };
            format!("\x1b[{n}~").into_bytes()
        }
        KeyCode::F(n) if n <= 4 && n > 0 => format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        KeyCode::F(n) => {
            let value = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return Vec::new(),
            };
            format!("\x1b[{value}~").into_bytes()
        }
        _ => Vec::new(),
    };
    if alt
        && matches!(
            event.code,
            KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace
        )
    {
        bytes.insert(0, 27);
    }
    bytes
}
