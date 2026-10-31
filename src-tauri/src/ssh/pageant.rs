//! PuTTY Pageant's local WM_COPYDATA/shared-memory SSH agent transport.
//! All native resources stay inside a bounded blocking request; never block the
//! async runtime or retain an HWND across requests. Mapping names are unguessable,
//! use the current user's default DACL/owner, and are never reused.
use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use windows::core::{s, PCSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM,
    WPARAM,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Memory::{
    CreateFileMappingA, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
    MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowA, SendMessageTimeoutA, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_COPYDATA,
};

use super::{pageant_packet_len, LocalAgentStream, PAGEANT_MAPPING_SIZE};

const PAGEANT_COPYDATA_ID: usize = 0x804e50ba;
const PAGEANT_TIMEOUT_MS: u32 = 5000;

fn find_pageant() -> io::Result<HWND> {
    // SAFETY: both arguments are static NUL-terminated strings.
    unsafe { FindWindowA(s!("Pageant"), s!("Pageant")) }
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "Pageant is not running"))
}

struct Mapping(HANDLE);
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns the valid mapping handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct View(MEMORY_MAPPED_VIEW_ADDRESS);
impl Drop for View {
    fn drop(&mut self) {
        // SAFETY: the full mapping is writable and remains mapped until this
        // guard drops; erase signatures/request material before releasing it.
        unsafe {
            for offset in 0..PAGEANT_MAPPING_SIZE {
                std::ptr::write_volatile(self.0.Value.cast::<u8>().add(offset), 0);
            }
            let _ = UnmapViewOfFile(self.0);
        }
    }
}

fn query(request: Vec<u8>) -> io::Result<Vec<u8>> {
    if request.len() < 4 || pageant_packet_len(request[..4].try_into().unwrap())? != request.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Pageant request",
        ));
    }
    let window = find_pageant()?;
    let name = std::ffi::CString::new(format!(
        "PageantRequest{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
    .unwrap();
    // SAFETY: INVALID_HANDLE_VALUE requests a pagefile-backed mapping. The
    // random ASCII name is NUL-terminated; NULL security attributes use the
    // current token's default security descriptor (non-inheritable handle).
    let mapping = Mapping(
        unsafe {
            CreateFileMappingA(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                PAGEANT_MAPPING_SIZE as u32,
                PCSTR(name.as_ptr().cast()),
            )
        }
        .map_err(io::Error::other)?,
    );
    // Never trust an existing mapping even in the astronomically unlikely
    // event of a random-name collision.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Pageant mapping name collision",
        ));
    }
    // SAFETY: map exactly the size of our newly-created object.
    let address =
        unsafe { MapViewOfFile(mapping.0, FILE_MAP_ALL_ACCESS, 0, 0, PAGEANT_MAPPING_SIZE) };
    if address.Value.is_null() {
        return Err(io::Error::last_os_error());
    }
    let view = View(address);
    // SAFETY: the newly mapped region is writable and larger than request.
    unsafe {
        std::ptr::copy_nonoverlapping(request.as_ptr(), view.0.Value.cast(), request.len());
    }
    let copy = COPYDATASTRUCT {
        dwData: PAGEANT_COPYDATA_ID,
        cbData: name.as_bytes_with_nul().len() as u32,
        lpData: name.as_ptr().cast_mut().cast(),
    };
    let mut response = 0;
    // SAFETY: WM_COPYDATA copies the descriptor/name synchronously; both stay
    // alive through the bounded call. Pageant opens the shared mapping itself.
    let sent = unsafe {
        SendMessageTimeoutA(
            window,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM((&copy as *const COPYDATASTRUCT) as isize),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            PAGEANT_TIMEOUT_MS,
            Some(&mut response),
        )
    };
    if sent.0 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Pageant did not respond within 5 seconds",
        ));
    }
    if response == 0 {
        return Err(io::Error::other("Pageant rejected the request"));
    }
    let mut header = [0; 4];
    // SAFETY: copy a fixed-size header, validate before reading the body. Never
    // form a mutable Rust slice over memory that the other process can access.
    unsafe {
        std::ptr::copy_nonoverlapping(view.0.Value.cast::<u8>(), header.as_mut_ptr(), 4);
    }
    let length = pageant_packet_len(&header)?;
    let mut packet = vec![0; length];
    unsafe {
        std::ptr::copy_nonoverlapping(view.0.Value.cast::<u8>(), packet.as_mut_ptr(), length);
    }
    // Reject a header changed between the bounded reads as well.
    if packet[..4] != header {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Pageant reply changed during read",
        ));
    }
    Ok(packet)
}

pub(super) async fn connect() -> io::Result<Box<dyn LocalAgentStream>> {
    find_pageant()?;
    let (client, mut server) = tokio::io::duplex(8192);
    tokio::spawn(async move {
        loop {
            let mut header = [0; 4];
            if server.read_exact(&mut header).await.is_err() {
                break;
            }
            let length = match pageant_packet_len(&header) {
                Ok(length) => length,
                Err(_) => break,
            };
            let mut request = vec![0; length];
            request[..4].copy_from_slice(&header);
            if server.read_exact(&mut request[4..]).await.is_err() {
                break;
            }
            let reply = match tokio::task::spawn_blocking(move || query(request)).await {
                Ok(Ok(reply)) => reply,
                Ok(Err(error)) => {
                    tracing::debug!("Pageant request failed: {error}");
                    break;
                }
                Err(_) => break,
            };
            if server.write_all(&reply).await.is_err() {
                break;
            }
        }
    });
    Ok(Box::new(client))
}
