use std::{cmp, io};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Mock {
    inner: Inner,
}

#[derive(Debug, Clone, Default)]
pub struct Builder {
    actions: VecDeque<Action>,
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
    state: Option<State>,
}

#[derive(Debug)]
enum State {
    Reading(Vec<u8>),
    Writing(Vec<u8>),
    Waiting(Instant),
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&mut self, buf: &[u8]) -> &mut Self {
        self.actions.push_back(Action::Read(buf.into()));
        self
    }

    pub fn write(&mut self, buf: &[u8]) -> &mut Self {
        self.actions.push_back(Action::Write(buf.into()));
        self
    }

    pub fn wait(&mut self, duration: Duration) -> &mut Self {
        let duration = cmp::max(duration, Duration::from_millis(1));
        self.actions.push_back(Action::Wait(duration));
        self
    }

    pub fn build<T>(&mut self) -> T
        where T: From<Self>
    {
        self.clone().into()
    }
}

impl From<Builder> for Mock {
    fn from(src: Builder) -> Mock {
        Mock {
            inner: Inner {
                actions: src.actions,
                state: None,
            },
        }
    }
}

impl Mock {
}

impl Inner {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        match self.action() {
            Some(&mut State::Reading(ref mut data)) =>{
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

    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        match self.action() {
            Some(&mut State::Writing(ref mut expect)) => {
                let n = cmp::min(src.len(), expect.len());

                // Assert the data matches
                assert_eq!(&src[..n], &expect[..n]);

                // Drop data that was matched
                expect.drain(..n);

                // Return the number of bytes matched
                Ok(n)
            }
            Some(_) => {
                // Expecting a read
                Err(io::ErrorKind::WouldBlock.into())
            }
            None => {
                // Socket is closed
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
    }

    fn remaining_wait(&mut self) -> Option<Duration> {
        match self.action() {
            Some(&mut State::Waiting(until)) => {
                let now = Instant::now();

                if now >= until {
                    return None;
                }

                Some(until - now)
            }
            _ => None,
        }
    }

    fn action(&mut self) -> Option<&mut State> {
        if self.is_current_action_complete() {
            // Clear the state
            self.state = None;
        }

        if self.state.is_none() {
            // Get the next action and prepare it
            match self.actions.pop_front() {
                Some(Action::Read(data)) => {
                    self.state = Some(State::Reading(data));
                }
                Some(Action::Write(data)) => {
                    self.state = Some(State::Writing(data));
                }
                Some(Action::Wait(dur)) => {
                    let until = Instant::now() + dur;
                    self.state = Some(State::Waiting(until));
                }
                None => {}
            }
        }

        self.state.as_mut()
    }

    fn is_current_action_complete(&mut self) -> bool {
        match self.state {
            Some(State::Reading(ref data)) => {
                data.is_empty()
            }
            Some(State::Writing(ref data)) => {
                data.is_empty()
            }
            Some(State::Waiting(until)) => {
                Instant::now() >= until
            }
            None => false,
        }
    }
}

impl io::Read for Mock {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
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
}

impl io::Write for Mock {
    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        match self.inner.write(src) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                panic!("mock_io::Mock not currently expecting a write");
            }
            ret => ret,
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


#[cfg(feature = "tokio")]
pub use tokio::*;

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

    #[derive(Debug)]
    pub struct AsyncMock {
        inner: ::Inner,
        timer: Timer,
        sleep: Option<Sleep>,
        read_wait: Option<Task>,
        write_wait: Option<Task>,
    }

    impl From<Builder> for AsyncMock {
        fn from(src: Builder) -> Self {
            // TODO: We probably want a higher resolution timer.
            let timer = tokio_timer::wheel()
                .tick_duration(Duration::from_millis(1))
                .max_timeout(Duration::from_secs(3600))
                .build();

            AsyncMock {
                inner: ::Inner {
                    actions: src.actions,
                    state: None,
                },
                timer: timer,
                sleep: None,
                read_wait: None,
                write_wait: None,
            }
        }
    }

    impl AsyncMock {
        fn maybe_wakeup_reader(&mut self) {
            match self.inner.action() {
                Some(&mut State::Reading(_)) | None => {
                    if let Some(task) = self.read_wait.take() {
                        task.notify();
                    }
                }
                _ => {}
            }
        }

        fn maybe_wakeup_writer(&mut self) {
            match self.inner.action() {
                Some(&mut State::Writing(_)) => {
                    if let Some(task) = self.write_wait.take() {
                        task.notify();
                    }
                }
                _ => {}
            }
        }
    }

    impl io::Read for AsyncMock {
        fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
            loop {
                if let Some(ref mut sleep) = self.sleep {
                    let res = try!(sleep.poll());

                    if !res.is_ready() {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                }

                // If a sleep is set, it has already fired
                self.sleep = None;

                match self.inner.read(dst) {
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if let Some(rem) = self.inner.remaining_wait() {
                            self.sleep = Some(self.timer.sleep(rem));
                        } else {
                            self.read_wait = Some(task::current());
                            return Err(io::ErrorKind::WouldBlock.into());
                        }
                    }
                    ret => {
                        self.maybe_wakeup_writer();
                        return ret;
                    }
                }
            }
        }
    }

    impl AsyncRead for AsyncMock {
    }

    impl io::Write for AsyncMock {
        fn write(&mut self, src: &[u8]) -> io::Result<usize> {
            match self.inner.write(src) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.write_wait = Some(task::current());
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                ret => {
                    self.maybe_wakeup_reader();
                    ret
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncWrite for AsyncMock {
        fn shutdown(&mut self) -> Poll<(), io::Error> {
            Ok(Async::Ready(()))
        }
    }
}
