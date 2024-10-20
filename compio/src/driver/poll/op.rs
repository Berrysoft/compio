use std::{io, pin::Pin, task::Poll};

use polling::Event;

pub use crate::driver::unix::op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IoBuf, IoBufMut},
    driver::{Decision, OpCode},
    op::*,
    syscall,
};

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_os = "illumos"
        )) {
            let fd = self.fd;
            let slice = self.buffer.as_uninit_slice();
            Ok(Decision::Completed(syscall!(pread(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                self.offset as _
            ))? as _))
        } else {
            Ok(Decision::wait_readable(self.fd))
        }
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd;
        let slice = self.buffer.as_uninit_slice();

        syscall!(
            break pread(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                self.offset as _
            )
        )
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_os = "illumos"
        )) {
            let slice = self.buffer.as_slice();
            Ok(Decision::Completed(syscall!(pwrite(
                self.fd,
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _
            ))? as _))
        } else {
            Ok(Decision::wait_writable(self.fd))
        }
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();

        syscall!(
            break pwrite(
                self.fd,
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _
            )
        )
    }
}

impl OpCode for Sync {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(fsync(self.fd))? as _))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("Sync operation should not be submitted to polling")
    }
}

impl OpCode for Accept {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            accept(
                self.fd,
                &mut self.buffer as *mut _ as *mut _,
                &mut self.addr_len
            ) or wait_readable(self.fd)
        )
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(
            break accept(
                self.fd,
                &mut self.buffer as *mut _ as *mut _,
                &mut self.addr_len
            )
        )
    }
}

impl OpCode for Connect {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            connect(self.fd, self.addr.as_ptr(), self.addr.len()) or wait_writable(self.fd)
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(getsockopt(
            self.fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut err as *mut _ as *mut _,
            &mut err_len
        ))?;

        let res = if err == 0 {
            Ok(0)
        } else {
            Err(io::Error::from_raw_os_error(err))
        };
        Poll::Ready(res)
    }
}

impl<T: AsIoSlicesMut + Unpin> OpCode for RecvImpl<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        syscall!(break readv(self.fd, self.slices.as_ptr() as _, self.slices.len() as _,))
    }
}

impl<T: AsIoSlices + Unpin> OpCode for SendImpl<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        self.slices = unsafe { self.buffer.as_io_slices() };
        syscall!(break writev(self.fd, self.slices.as_ptr() as _, self.slices.len() as _,))
    }
}

impl<T: AsIoSlicesMut + Unpin> OpCode for RecvFromImpl<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.set_msg();
        syscall!(recvmsg(self.fd, &mut self.msg, 0) or wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break recvmsg(self.fd, &mut self.msg, 0))
    }
}

impl<T: AsIoSlices + Unpin> OpCode for SendToImpl<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.set_msg();
        syscall!(sendmsg(self.fd, &self.msg, 0) or wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break sendmsg(self.fd, &self.msg, 0))
    }
}
