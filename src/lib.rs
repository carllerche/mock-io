//! A mock type implementing [`Read`] and [`Write`].
//!
//! # Overview
//!
//! Provides a type that implements [`Read`] + [`Write`] that can be configured
//! to handle an arbitrary sequence of read and write operations. This is useful
//! for writing unit tests for networking services as using an actual network
//! type is fairly non deterministic.
//!
//! # Usage
//!
//! Add the following to your `Cargo.toml`
//!
//! ```toml
//! [dependencies]
//! mock-io = { git = "https://github.com/carllerche/mock-io" }
//! ```
//!
//! Then use it in your project. For example, a test could be written:
//!
//! ```
//! extern crate mock_io;
//!
//! use mock_io::{Builder, Mock};
//! use std::io::{Read, Write};
//!
//! # /*
//! #[test]
//! # */
//! fn test_io() {
//!     let mut mock = Builder::new()
//!         .write(b"ping")
//!         .read(b"pong")
//!         .build();
//!
//!    let n = mock.write(b"ping").unwrap();
//!    assert_eq!(n, 4);
//!
//!    let mut buf = vec![];
//!    mock.read_to_end(&mut buf).unwrap();
//!
//!    assert_eq!(buf, b"pong");
//! }
//! # pub fn main() {
//! # test_io();
//! # }
//! ```
//!
//! Attempting to write data that the mock isn't expected will result in a
//! panic.
//!
//! # Tokio
//!
//! `Mock` also supports tokio by implementing `AsyncRead` and `AsyncWrite`.
//! When using `Mock` in context of a Tokio task, it will automatically switch
//! to "async" behavior (this can also be set explicitly by calling `set_async`
//! on `Builder`).
//!
//! In async mode, calls to read and write are non-blocking and the task using
//! the mock is notified when the readiness state changes.
//!
//! # `io-dump` dump files
//!
//! `Mock` can also be configured from an `io-dump` file. By doing this, the
//! mock value will replay a previously recorded behavior. This is useful for
//! collecting a scenario from the real world and replying it as part of a test.
//!
//! [`Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
//! [`Write`]: https://doc.rust-lang.org/std/io/trait.Write.html

use std::{cmp, io};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// An I/O handle that follows a predefined script.
///
/// This value is created by `Builder` and implements `Read + `Write`. It
/// follows the scenario described by the builder and panics otherwise.
#[derive(Debug)]
pub struct Mock {
    inner: Inner,
    tokio: TokioInner,
    async: Option<bool>,
}

/// Builds `Mock` instances.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    // Sequence of actions for the Mock to take
    actions: VecDeque<Action>,

    // true for Tokio, false for blocking, None to auto detect
    async: Option<bool>,
}

#[derive(Debug, Clone)]
enum Action {
    Read(Vec<u8>),
    Write(Vec<u8>),
    Wait(Duration),
}

#[derive(Debug)]
struct Inner {
    actions: VecDeque<Action>,
    waiting: Option<Instant>,
}

impl Builder {
    /// Return a new, empty `Builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sequence a `read` operation.
    ///
    /// The next operation in the mock's script will be to expect a `read` call
    /// and return `buf`.
    pub fn read(&mut self, buf: &[u8]) -> &mut Self {
        self.actions.push_back(Action::Read(buf.into()));
        self
    }

    /// Sequence a `write` operation.
    ///
    /// The next operation in the mock's script will be to expect a `write`
    /// call.
    pub fn write(&mut self, buf: &[u8]) -> &mut Self {
        self.actions.push_back(Action::Write(buf.into()));
        self
    }

    /// Sequence a wait.
    ///
    /// The next operation in the mock's script will be to wait without doing so
    /// for `duration` amount of time.
    pub fn wait(&mut self, duration: Duration) -> &mut Self {
        let duration = cmp::max(duration, Duration::from_millis(1));
        self.actions.push_back(Action::Wait(duration));
        self
    }

    /// Build a `Mock` value according to the defined script.
    pub fn build(&mut self) -> Mock {
        self.clone().into()
    }
}

impl From<Builder> for Mock {
    fn from(src: Builder) -> Mock {
        Mock {
            inner: Inner {
                actions: src.actions,
                waiting: None,
            },
            tokio: TokioInner::default(),
            async: src.async,
        }
    }
}

impl Mock {
    fn sync_read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        use std::thread;

        loop {
            match self.inner.read(dst) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if let Some(rem) = self.inner.remaining_wait() {
                        thread::sleep(rem);
                    } else {
                        // We've entered a dead lock scenario. The peer expects
                        // a write but we are reading.
                        panic!("mock_io::Mock expects write but currently blocked in read");
                    }
                }
                ret => return ret,
            }
        }
    }

    fn sync_write(&mut self, src: &[u8]) -> io::Result<usize> {
        match self.inner.write(src) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                panic!("mock_io::Mock not currently expecting a write");
            }
            ret => ret,
        }
    }

    /// Returns `true` if running in a futures-rs task context
    fn is_async(&self) -> bool {
        self.async.unwrap_or(tokio::is_task_ctx())
    }
}

impl Inner {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        match self.action() {
            Some(&mut Action::Read(ref mut data)) =>{
                // Figure out how much to copy
                let n = cmp::min(dst.len(), data.len());

                // Copy the data into the `dst` slice
                (&mut dst[..n]).copy_from_slice(&data[..n]);

                // Drain the data from the source
                data.drain(..n);

                // Return the number of bytes read
                Ok(n)
            }
            Some(_) => {
                // Either waiting or expecting a write
                Err(io::ErrorKind::WouldBlock.into())
            }
            None => {
                 Ok(0)
            }
        }
    }

    fn write(&mut self, mut src: &[u8]) -> io::Result<usize> {
        let mut ret = 0;

        if self.actions.is_empty() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }

        for i in 0..self.actions.len() {
            match self.actions[i] {
                Action::Write(ref mut expect) => {
                    let n = cmp::min(src.len(), expect.len());

                    assert_eq!(&src[..n], &expect[..n]);

                    // Drop data that was matched
                    expect.drain(..n);
                    src = &src[n..];

                    ret += n;

                    if src.is_empty() {
                        return Ok(ret);
                    }
                }
                _ => {}
            }

            // TODO: remove write
        }

        if ret == 0 {
            panic!("not expecting write");
        }

        Ok(ret)
    }

    fn remaining_wait(&mut self) -> Option<Duration> {
        match self.action() {
            Some(&mut Action::Wait(dur)) => Some(dur),
            _ => None,
        }
    }

    fn action(&mut self) -> Option<&mut Action> {
        loop {
            if self.actions.is_empty() {
                return None;
            }

            match self.actions[0] {
                Action::Read(ref mut data) => {
                    if !data.is_empty() {
                        break;
                    }
                }
                Action::Write(ref mut data) => {
                    if !data.is_empty() {
                        break;
                    }
                }
                Action::Wait(ref mut dur) => {
                    if let Some(until) = self.waiting {
                        let now = Instant::now();

                        if now < until {
                            break;
                        }
                    } else {
                        self.waiting = Some(Instant::now() + *dur);
                        break;
                    }
                }
            }

            self.actions.pop_front();
        }

        self.actions.front_mut()
    }
}

impl io::Read for Mock {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        if self.is_async() {
            async_read(self, dst)
        } else {
            self.sync_read(dst)
        }
    }
}

impl io::Write for Mock {
    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        if self.is_async() {
            async_write(self, src)
        } else {
            self.sync_write(src)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "io-dump")]
mod io_dump {
    extern crate io_dump;

    use Builder;
    use self::io_dump::{Packets, Direction};

    use std::io;
    use std::path::Path;
    use std::time::Duration;

    impl Builder {
        /// Open an `io-dump` file and return a new `Builder` representing the
        /// order of operations defined in the file.
        pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
            let file = try!(io_dump::open(path));
            Ok(Self::from_packets(file))
        }

        /// Return a new `Builder` set to reply the order of operations defined
        /// in `Packets`.
        pub fn from_packets<T: io::Read>(packets: Packets<T>) -> Self {
            let mut ret = Builder::new();
            let mut last = Duration::from_millis(0);

            for packet in packets {
                match packet.direction() {
                    Direction::Read => {
                        let wait = packet.elapsed() - last;

                        ret.wait(wait);
                        ret.read(packet.data());
                    }
                    Direction::Write => {
                        ret.write(packet.data());
                    }
                }

                last = packet.elapsed();
            }

            ret
        }
    }
}

use tokio::*;

#[cfg(feature = "tokio")]
mod tokio {
    extern crate futures;
    extern crate tokio_io;
    extern crate tokio_timer;

    use super::*;

    use self::futures::{Future, Poll, Async};
    use self::futures::task::{self, Task};
    use self::tokio_io::{AsyncRead, AsyncWrite};
    use self::tokio_timer::{Timer, Sleep};

    use std::io;

    impl Builder {
        pub fn set_async(&mut self, is_async: bool) -> &mut Self {
            self.async = Some(is_async);
            self
        }
    }

    #[derive(Debug)]
    pub struct TokioInner {
        timer: Timer,
        sleep: Option<Sleep>,
        read_wait: Option<Task>,
    }

    impl Default for TokioInner {
        fn default() -> Self {
            // TODO: We probably want a higher resolution timer.
            let timer = tokio_timer::wheel()
                .tick_duration(Duration::from_millis(1))
                .max_timeout(Duration::from_secs(3600))
                .build();

            TokioInner {
                timer: timer,
                sleep: None,
                read_wait: None,
            }
        }
    }

    impl Mock {
        fn maybe_wakeup_reader(&mut self) {
            match self.inner.action() {
                Some(&mut Action::Read(_)) | None => {
                    if let Some(task) = self.tokio.read_wait.take() {
                        task.notify();
                    }
                }
                _ => {}
            }
        }
    }

    pub fn async_read(me: &mut Mock, dst: &mut [u8]) -> io::Result<usize> {
        loop {
            if let Some(ref mut sleep) = me.tokio.sleep {
                let res = try!(sleep.poll());

                if !res.is_ready() {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
            }

            // If a sleep is set, it has already fired
            me.tokio.sleep = None;

            match me.inner.read(dst) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if let Some(rem) = me.inner.remaining_wait() {
                        me.tokio.sleep = Some(me.tokio.timer.sleep(rem));
                    } else {
                        me.tokio.read_wait = Some(task::current());
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                }
                ret => return ret,
            }
        }
    }

    pub fn async_write(me: &mut Mock, src: &[u8]) -> io::Result<usize> {
        match me.inner.write(src) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                panic!("unexpected WouldBlock");
            }
            ret => {
                me.maybe_wakeup_reader();
                ret
            }
        }
    }

    impl AsyncRead for Mock {
    }

    impl AsyncWrite for Mock {
        fn shutdown(&mut self) -> Poll<(), io::Error> {
            Ok(Async::Ready(()))
        }
    }

    /// Returns `true` if called from the context of a futures-rs Task
    pub fn is_task_ctx() -> bool {
        use std::panic;

        // Save the existing panic hook
        let h = panic::take_hook();

        // Install a new one that does nothing
        panic::set_hook(Box::new(|_| {}));

        // Attempt to call the fn
        let r = panic::catch_unwind(|| task::current()).is_ok();

        // Re-install the old one
        panic::set_hook(h);

        // Return the result
        r
    }
}

#[cfg(not(feature = "tokio"))]
mod tokio {
    use Mock;

    use std::io;

    #[derive(Debug, Default)]
    pub struct TokioInner;

    pub fn async_read(_: &mut Mock, _: &mut [u8]) -> io::Result<usize> {
        unreachable!();
    }

    pub fn async_write(_: &mut Mock, _: &[u8]) -> io::Result<usize> {
        unreachable!();
    }

    pub fn is_task_ctx() -> bool {
        false
    }
}
