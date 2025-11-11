use eyre::{Result, eyre};

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
}
