extern crate mock_io;

use mock_io::{Mock, Builder};

use std::io::{Read, Write};
use std::time::{Duration, Instant};

#[test]
fn test_empty_mock() {
    let mut mock: Mock = Builder::new().build();

    let mut buf = [0; 1024];
    assert_eq!(0, mock.read(&mut buf).unwrap());

    let mut mock: Mock = Builder::new().build();
    assert!(mock.write(b"hello").is_err());
}

#[test]
fn test_single_read() {
    let mut mock: Mock = Builder::new()
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
fn test_blocking_read() {
    let dur = Duration::from_millis(100);

    let mut mock: Mock = Builder::new()
        .wait(dur)
        .read(b"hello world")
        .build();

    let mut buf = [0; 1024];
    let now = Instant::now();

    assert_eq!(11, mock.read(&mut buf).unwrap());

    assert!(now.elapsed() >= dur);

    assert_eq!(&buf[..11], b"hello world");
}

#[test]
fn test_partial_read() {
    let mut mock: Mock = Builder::new()
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
fn test_single_write() {
    let mut mock: Mock = Builder::new()
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
    let mut mock: Mock = Builder::new()
        .write(b"hello world")
        .build();

    let mut buf = [0; 1024];

    assert_eq!(6, mock.write(b"hello ").unwrap());
    assert_eq!(5, mock.write(b"world").unwrap());

    // Empty
    assert_eq!(0, mock.read(&mut buf).unwrap());
    assert!(mock.write(b"hello").is_err());
}
