#![cfg(feature = "tokio")]

extern crate mock_io;

extern crate futures;
extern crate tokio_io;

use mock_io::{Mock, Builder};

use futures::{Future, Poll, Async};
use tokio_io::io::read_to_end;

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

#[test]
fn test_empty_mock() {
    let mut mock = Builder::new().build();

    let mut buf = [0; 1024];
    assert_eq!(0, mock.read(&mut buf).unwrap());

    let mut mock = Builder::new().build();
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_single_read() {
    let mut mock = Builder::new()
        .read(b"hello world")
        .build();

    let mut buf = [0; 1024];

    assert_eq!(11, mock.read(&mut buf).unwrap());
    assert_eq!(&buf[..11], b"hello world");

    // Mock is empty
    assert_eq!(0, mock.read(&mut buf).unwrap());
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_async_read() {
    let dur = Duration::from_millis(100);

    let mock = Builder::new()
        .wait(dur)
        .read(b"hello world")
        .build();

    let now = Instant::now();

    let (mut mock, buf) = read_to_end(mock, vec![]).wait().unwrap();

    assert!(now.elapsed() >= dur);
    assert_eq!(11, buf.len());

    assert_eq!(&buf[..], b"hello world");
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_partial_read() {
    let mut mock = Builder::new()
        .read(b"hello world")
        .build();

    let mut buf = [0; 6];

    assert_eq!(6, mock.read(&mut buf).unwrap());
    assert_eq!(&buf[..], b"hello ");

    assert_eq!(5, mock.read(&mut buf).unwrap());
    assert_eq!(&buf[..5], b"world");

    // Mock is empty
    assert_eq!(0, mock.read(&mut buf).unwrap());
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_read_when_expect_write() {
    struct MyFuture {
        mock: Mock,
        wrote: bool,
    };

    let mock = Builder::new()
        .write(b"hello world")
        .read(b"ping")
        .build();

    let fut = MyFuture {
        mock: mock,
        wrote: false,
    };

    impl Future for MyFuture {
        type Item = ();
        type Error = io::Error;

        fn poll(&mut self) -> Poll<(), io::Error> {
            let mut buf = [0; 128];

            // Try reading
            match self.mock.read(&mut buf) {
                Ok(n) => {
                    assert!(self.wrote);
                    assert_eq!(4, n);
                    assert_eq!(&buf[..n], b"ping");
                    return Ok(Async::Ready(()));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }

            // Write
            assert_eq!(11, self.mock.write(b"hello world").unwrap());
            self.wrote = true;

            Ok(Async::NotReady)
        }
    }

    fut.wait().unwrap();
}

#[test]
fn test_single_write() {
    let mut mock = Builder::new()
        .write(b"hello world")
        .build();

    let mut buf = [0; 1024];

    assert_eq!(11, mock.write(b"hello world").unwrap());

    // Empty
    assert_eq!(0, mock.read(&mut buf).unwrap());
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_partial_write() {
    let mut mock = Builder::new()
        .write(b"hello world")
        .build();

    let mut buf = [0; 1024];

    assert_eq!(6, mock.write(b"hello ").unwrap());
    assert_eq!(5, mock.write(b"world").unwrap());

    // Empty
    assert_eq!(0, mock.read(&mut buf).unwrap());
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_write_when_expecting_read() {
    let mut mock = Builder::new()
        .read(b"pong")
        .write(b"ping")
        .build();

    let mut buf = [0; 1024];

    assert_eq!(4, mock.write(b"ping").unwrap());
    assert_eq!(4, mock.read(&mut buf).unwrap());

    assert_eq!(&buf[..4], b"pong");
}
