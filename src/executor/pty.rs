use eyre::{Result, eyre};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[cfg(unix)]
use nix::libc;
#[cfg(unix)]
use nix::pty::{Winsize, openpty};
#[cfg(unix)]
use nix::sys::termios::{self, Termios};
#[cfg(unix)]
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};
#[cfg(unix)]
use tokio::io::unix::AsyncFd;

/// Interactive PTY for handling terminal I/O with proper TTY support
#[cfg(unix)]
#[derive(Debug)]
pub struct InteractivePty {
    master_fd: OwnedFd,
    slave_fd: OwnedFd,
    original_termios: Option<Termios>,
}

#[cfg(unix)]
impl InteractivePty {
    /// Create a new PTY pair and set terminal to raw mode
    pub fn new() -> Result<Self> {
        // Open PTY pair
        let pty_result = openpty(None, None)?;

        let master_fd = pty_result.master;
        let slave_fd = pty_result.slave;

        // Save original terminal settings from stdin
        let original_termios = if atty::is(atty::Stream::Stdin) {
            let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
            match termios::tcgetattr(stdin_fd) {
                Ok(termios_val) => {
                    // Set stdin to raw mode for interactive input
                    let mut raw = termios_val.clone();
                    termios::cfmakeraw(&mut raw);
                    if let Err(e) = termios::tcsetattr(stdin_fd, termios::SetArg::TCSANOW, &raw) {
                        eprintln!("Warning: Failed to set terminal to raw mode: {}", e);
                    }
                    Some(termios_val)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to get terminal attributes: {}", e);
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            master_fd,
            slave_fd,
            original_termios,
        })
    }

    /// Get the master file descriptor as raw fd
    pub fn master_fd(&self) -> RawFd {
        self.master_fd.as_raw_fd()
    }

    /// Get the slave file descriptor as raw fd
    pub fn slave_fd(&self) -> RawFd {
        self.slave_fd.as_raw_fd()
    }

    /// Get borrowed master fd
    pub fn master_as_fd(&self) -> BorrowedFd<'_> {
        self.master_fd.as_fd()
    }

    /// Get borrowed slave fd
    pub fn slave_as_fd(&self) -> BorrowedFd<'_> {
        self.slave_fd.as_fd()
    }

    /// Restore original terminal settings
    pub fn restore_terminal(&self) -> Result<()> {
        if let Some(ref termios_val) = self.original_termios {
            let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
            termios::tcsetattr(stdin_fd, termios::SetArg::TCSANOW, termios_val)
                .map_err(|e| eyre!("Failed to restore terminal: {}", e))?;
        }
        Ok(())
    }

    /// Set the window size for the PTY
    pub fn set_window_size(&self, rows: u16, cols: u16) -> Result<()> {
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        nix::ioctl_write_ptr_bad!(set_winsize, libc::TIOCSWINSZ, Winsize);
        unsafe {
            set_winsize(self.master_fd.as_raw_fd(), &winsize).map_err(|e| eyre!("Failed to set window size: {}", e))?;
        }
        Ok(())
    }

    /// Get current terminal window size
    pub fn get_terminal_size() -> Result<(u16, u16)> {
        if !atty::is(atty::Stream::Stdin) {
            return Ok((24, 80)); // Default size
        }

        nix::ioctl_read_bad!(get_winsize, libc::TIOCGWINSZ, Winsize);
        let mut winsize = Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        unsafe {
            get_winsize(libc::STDIN_FILENO, &mut winsize).map_err(|e| eyre!("Failed to get terminal size: {}", e))?;
        }

        Ok((winsize.ws_row, winsize.ws_col))
    }
}

#[cfg(unix)]
impl Drop for InteractivePty {
    fn drop(&mut self) {
        // Always try to restore terminal on drop
        if let Err(e) = self.restore_terminal() {
            eprintln!("Warning: Failed to restore terminal in Drop: {}", e);
        }

        // File descriptors will be automatically closed when OwnedFd is dropped
    }
}

/// I/O Proxy for bidirectional communication with logging
#[cfg(unix)]
#[derive(Debug)]
pub struct PtyIoProxy {
    log_file: Arc<Mutex<tokio::fs::File>>,
}

#[cfg(unix)]
impl PtyIoProxy {
    /// Create a new I/O proxy with logging to the specified file
    pub async fn new(log_path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Open log file for appending
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .map_err(|e| eyre!("Failed to open log file {:?}: {}", log_path, e))?;

        Ok(Self {
            log_file: Arc::new(Mutex::new(log_file)),
        })
    }

    /// Run the I/O proxy loop: stdin -> PTY master and PTY master -> stdout
    /// Returns when either stream closes or an error occurs
    pub async fn run_proxy(&self, master_fd: RawFd) -> Result<()> {
        // Create async file descriptor for the PTY master
        let async_master = AsyncFd::new(master_fd)?;

        // Create async stdin/stdout
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();

        let log_file_stdin = self.log_file.clone();
        let log_file_stdout = self.log_file.clone();

        // Buffer for I/O operations
        let mut stdin_buf = vec![0u8; 8192];
        let mut master_buf = vec![0u8; 8192];

        // Spawn task for stdin -> master
        let stdin_to_master = {
            let async_master_write = AsyncFd::new(master_fd)?;
            tokio::spawn(async move {
                loop {
                    match stdin.read(&mut stdin_buf).await {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            let data = &stdin_buf[..n];

                            // Write to PTY master
                            let mut guard = async_master_write.writable().await?;
                            match guard.try_io(|_| {
                                let written =
                                    unsafe { libc::write(master_fd, data.as_ptr() as *const libc::c_void, data.len()) };
                                if written < 0 {
                                    Err(std::io::Error::last_os_error())
                                } else {
                                    Ok(written as usize)
                                }
                            }) {
                                Ok(Ok(_)) => {
                                    // Log input
                                    let mut log = log_file_stdin.lock().await;
                                    let _ = log.write_all(data).await;
                                    let _ = log.flush().await;
                                }
                                Ok(Err(e)) => return Err(eyre!("Write to PTY failed: {}", e)),
                                Err(_would_block) => continue,
                            }
                        }
                        Err(e) => return Err(eyre!("Read from stdin failed: {}", e)),
                    }
                }
                Ok::<(), eyre::Report>(())
            })
        };

        // Spawn task for master -> stdout
        let master_to_stdout = tokio::spawn(async move {
            loop {
                let mut guard = async_master.readable().await?;
                match guard.try_io(|_| {
                    let read = unsafe {
                        libc::read(
                            master_fd,
                            master_buf.as_mut_ptr() as *mut libc::c_void,
                            master_buf.len(),
                        )
                    };
                    if read < 0 {
                        Err(std::io::Error::last_os_error())
                    } else if read == 0 {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "PTY master closed",
                        ))
                    } else {
                        Ok(read as usize)
                    }
                }) {
                    Ok(Ok(n)) => {
                        let data = &master_buf[..n];

                        // Write to stdout
                        stdout.write_all(data).await?;
                        stdout.flush().await?;

                        // Log output
                        let mut log = log_file_stdout.lock().await;
                        let _ = log.write_all(data).await;
                        let _ = log.flush().await;
                    }
                    Ok(Err(e)) => {
                        if e.kind() == std::io::ErrorKind::UnexpectedEof {
                            break;
                        }
                        return Err(eyre!("Read from PTY failed: {}", e));
                    }
                    Err(_would_block) => continue,
                }
            }
            Ok::<(), eyre::Report>(())
        });

        // Wait for either task to complete
        tokio::select! {
            result = stdin_to_master => {
                result??;
            }
            result = master_to_stdout => {
                result??;
            }
        }

        Ok(())
    }
}

// Windows stub - PTY support not implemented yet
#[cfg(not(unix))]
pub struct InteractivePty;

#[cfg(not(unix))]
impl InteractivePty {
    pub fn new() -> Result<Self> {
        Err(eyre!(
            "Interactive PTY support is not available on this platform. Only Unix-like systems (Linux, macOS) are currently supported."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_pty_creation() {
        // This test will be skipped in CI without TTY
        if !atty::is(atty::Stream::Stdin) {
            eprintln!("Skipping PTY test: no TTY available");
            return;
        }

        let result = InteractivePty::new();
        assert!(result.is_ok(), "Failed to create PTY: {:?}", result);

        if let Ok(pty) = result {
            assert!(pty.master_fd() >= 0, "Invalid master FD");
            assert!(pty.slave_fd() >= 0, "Invalid slave FD");
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_terminal_restoration() {
        if !atty::is(atty::Stream::Stdin) {
            eprintln!("Skipping terminal restoration test: no TTY available");
            return;
        }

        let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
        let original = termios::tcgetattr(&stdin_fd).unwrap();

        {
            let pty = InteractivePty::new().unwrap();
            // PTY should modify terminal
            drop(pty); // Should restore on drop
        }

        let restored = termios::tcgetattr(&stdin_fd).unwrap();

        // Check that terminal was restored (at least some flags should match)
        assert_eq!(
            original.input_flags, restored.input_flags,
            "Terminal not properly restored"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_get_terminal_size() {
        let result = InteractivePty::get_terminal_size();
        assert!(result.is_ok(), "Failed to get terminal size");

        if let Ok((rows, cols)) = result {
            // Sanity check: terminal should have reasonable size
            assert!(rows > 0 && rows < 1000, "Invalid row count: {}", rows);
            assert!(cols > 0 && cols < 1000, "Invalid column count: {}", cols);
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn test_pty_not_supported_on_windows() {
        let result = InteractivePty::new();
        assert!(result.is_err(), "PTY should not be available on non-Unix platforms");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_pty_io_proxy_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let result = PtyIoProxy::new(log_path.clone()).await;
        assert!(result.is_ok(), "Failed to create PtyIoProxy: {:?}", result);

        // Verify log file was created
        assert!(log_path.exists(), "Log file should be created");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_pty_io_proxy_log_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path().join("nested/dir/test.log");

        // Should create parent directories
        let proxy = PtyIoProxy::new(log_path.clone()).await.unwrap();

        // Write some test data directly to log
        {
            let mut log = proxy.log_file.lock().await;
            log.write_all(b"test data\n").await.unwrap();
            log.flush().await.unwrap();
        }

        // Verify data was written
        let content = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert_eq!(content, "test data\n");
    }
}
