use std::any::Any;
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::slice;

use anyhow::{Result, anyhow};
use libc::{c_int, c_ulong, c_void};
use mozjpeg_sys as ffi;

#[derive(Debug)]
struct JpegError(c_int);

struct State {
    decompress: ffi::jpeg_decompress_struct,
    compress: ffi::jpeg_compress_struct,
    decompress_created: bool,
    compress_created: bool,
}

impl State {
    fn new() -> Self {
        // The libjpeg API requires these C structs to be zero-initialized before
        // their respective create functions are called.
        Self {
            decompress: unsafe { mem::zeroed() },
            compress: unsafe { mem::zeroed() },
            decompress_created: false,
            compress_created: false,
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // SAFETY: each structure is destroyed only when its create call
        // completed successfully, and State owns both structures.
        unsafe {
            if self.compress_created {
                ffi::jpeg_destroy_compress(&mut self.compress);
            }
            if self.decompress_created {
                ffi::jpeg_destroy_decompress(&mut self.decompress);
            }
        }
    }
}

struct OutputBuffer {
    pointer: *mut u8,
    size: c_ulong,
}

impl OutputBuffer {
    fn new() -> Self {
        Self {
            pointer: ptr::null_mut(),
            size: 0,
        }
    }

    fn copy_to_vec(&self) -> Result<Vec<u8>> {
        if self.pointer.is_null() || self.size == 0 {
            return Err(anyhow!("mozjpeg produced no output"));
        }
        let length =
            usize::try_from(self.size).map_err(|_| anyhow!("optimized JPEG is too large"))?;
        // SAFETY: jpeg_mem_dest returned a malloc-allocated buffer containing
        // `size` initialized bytes, which remains valid for this object's life.
        Ok(unsafe { slice::from_raw_parts(self.pointer, length) }.to_vec())
    }
}

impl Drop for OutputBuffer {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: jpeg_mem_dest transfers ownership of this malloc-allocated
            // buffer to the caller after compression.
            unsafe { libc::free(self.pointer.cast::<c_void>()) };
        }
    }
}

unsafe extern "C-unwind" fn error_exit(info: &mut ffi::jpeg_common_struct) {
    let code = if info.err.is_null() {
        -1
    } else {
        // SAFETY: libjpeg invokes the callback with its active error manager.
        unsafe { (*info.err).msg_code }
    };
    // `resume_unwind` deliberately skips the panic hook. The unwind is used as
    // libjpeg's non-local error return and is caught immediately by `transcode`.
    panic::resume_unwind(Box::new(JpegError(code)));
}

pub(super) fn transcode(input: &[u8]) -> Result<Vec<u8>> {
    let input_size: c_ulong = input
        .len()
        .try_into()
        .map_err(|_| anyhow!("JPEG is too large for mozjpeg's memory API"))?;

    let mut source_error: ffi::jpeg_error_mgr = unsafe { mem::zeroed() };
    let mut destination_error: ffi::jpeg_error_mgr = unsafe { mem::zeroed() };
    let mut output = OutputBuffer::new();
    // Declared last so State is dropped before the error managers and output.
    let mut state = State::new();

    let operation = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        ffi::jpeg_std_error(&mut source_error).error_exit = Some(error_exit);
        state.decompress.common.err = &mut source_error;
        ffi::jpeg_create_decompress(&mut state.decompress);
        state.decompress_created = true;
        ffi::jpeg_mem_src(&mut state.decompress, input.as_ptr(), input_size);

        ffi::jpeg_read_header(&mut state.decompress, 1);
        let coefficients = ffi::jpeg_read_coefficients(&mut state.decompress);

        ffi::jpeg_std_error(&mut destination_error).error_exit = Some(error_exit);
        state.compress.common.err = &mut destination_error;
        ffi::jpeg_create_compress(&mut state.compress);
        state.compress_created = true;
        ffi::jpeg_mem_dest(&mut state.compress, &mut output.pointer, &mut output.size);

        ffi::jpeg_copy_critical_parameters(&state.decompress, &mut state.compress);
        ffi::jpeg_simple_progression(&mut state.compress);
        state.compress.optimize_coding = 1;
        state.compress.write_JFIF_header = 0;
        state.compress.write_Adobe_marker = 0;
        ffi::jpeg_write_coefficients(&mut state.compress, coefficients);

        ffi::jpeg_finish_compress(&mut state.compress);
        ffi::jpeg_finish_decompress(&mut state.decompress);
    }));

    match operation {
        Ok(()) => output.copy_to_vec(),
        Err(payload) => Err(jpeg_error_from_panic(payload)),
    }
}

fn jpeg_error_from_panic(payload: Box<dyn Any + Send>) -> anyhow::Error {
    match payload.downcast::<JpegError>() {
        Ok(error) => anyhow!(
            "invalid or unsupported JPEG (mozjpeg error code {})",
            error.0
        ),
        Err(payload) => panic::resume_unwind(payload),
    }
}
