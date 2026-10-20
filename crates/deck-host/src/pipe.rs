//! Current user SID, pipe name, current-user-only pipe server, client connect, single-instance mutex.

use std::ffi::c_void;
use std::io::Error;
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};

const ERROR_PIPE_BUSY: i32 = 231;

fn win(e: windows::core::Error) -> Error {
    Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

/// String form of the current user's SID, e.g. `S-1-5-21-...`.
pub fn current_user_sid() -> std::io::Result<String> {
    // SAFETY: `tok` is a valid out-handle, closed on every path after a successful open.
    // The buffer is sized by the first call and TOKEN_USER is read from it only after the
    // second call succeeded; it is a Vec<u64> so it is aligned for TOKEN_USER and outlives
    // the read. The string from ConvertSidToStringSidW is freed once with LocalFree.
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).map_err(win)?;
        let r = (|| {
            let mut len = 0u32;
            let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut len); // sizing call, fails by design
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            GetTokenInformation(
                tok,
                TokenUser,
                Some(buf.as_mut_ptr() as *mut c_void),
                len,
                &mut len,
            )
            .map_err(win)?;
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut s).map_err(win)?;
            let out = s
                .to_string()
                .map_err(|e| Error::new(std::io::ErrorKind::InvalidData, e));
            let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
            out
        })();
        let _ = CloseHandle(tok);
        r
    }
}

/// `\\.\pipe\claude-deck-<sid>`
pub fn pipe_name(sid: &str) -> String {
    format!(r"\\.\pipe\claude-deck-{sid}")
}

/// Owns a security descriptor allowing only one SID (SDDL `O:<SID>D:P(A;;GA;;;<SID>)`, so the owner is explicit), freed on drop.
pub struct PipeSecurity {
    sd: PSECURITY_DESCRIPTOR,
    sa: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor and attributes are never mutated after creation and the raw pointer
// refers to memory owned by this struct until Drop, so sharing/sending across threads is fine.
unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    pub fn only(sid: &str) -> std::io::Result<Self> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})\0").encode_utf16().collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` is NUL-terminated and outlives the call; `sd` is a valid out-pointer.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .map_err(win)?;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        Ok(Self { sd, sa })
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: `sd` was allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW
        // (LocalAlloc) and is freed exactly once here.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.sd.0)));
        }
    }
}

/// `first`: first_pipe_instance(true) for the very first instance (anti-squatting).
/// Always rejects remote clients.
pub fn create_server(
    name: &str,
    sec: &PipeSecurity,
    first: bool,
) -> std::io::Result<NamedPipeServer> {
    // SAFETY: the pointer refers to a SECURITY_ATTRIBUTES (with a valid descriptor) owned by
    // `sec`, which outlives this call; CreateNamedPipe only reads it (const parameter).
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                &sec.sa as *const SECURITY_ATTRIBUTES as *mut c_void,
            )
    }
}

/// Opens the pipe, retrying ERROR_PIPE_BUSY every 50 ms up to `wait`; other errors return at once.
pub async fn connect_client(name: &str, wait: Duration) -> std::io::Result<NamedPipeClient> {
    let deadline = Instant::now() + wait;
    loop {
        match ClientOptions::new().open(name) {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

/// Whether the kernel object behind `handle` (a pipe) is owned by `sid` (case-insensitive).
pub fn server_owned_by(handle: &impl AsRawHandle, sid: &str) -> std::io::Result<bool> {
    // SAFETY: out-pointers are valid; the descriptor (which owns `owner`) is freed once after
    // the SID string was copied; the SID string is freed once.
    unsafe {
        let mut owner = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(
            HANDLE(handle.as_raw_handle()),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut sd),
        )
        .ok()
        .map_err(win)?;
        let mut s = PWSTR::null();
        let r = ConvertSidToStringSidW(owner, &mut s)
            .map_err(win)
            .and_then(|()| {
                s.to_string()
                    .map_err(|e| Error::new(std::io::ErrorKind::InvalidData, e))
            });
        if !s.is_null() {
            let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
        }
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        r.map(|o| o.eq_ignore_ascii_case(sid))
    }
}

/// `connect_client`, then refuses a server whose owner is not `sid` (PermissionDenied).
pub async fn connect_trusted(
    name: &str,
    wait: Duration,
    sid: &str,
) -> std::io::Result<NamedPipeClient> {
    let client = connect_client(name, wait).await?;
    if server_owned_by(&client, sid)? {
        Ok(client)
    } else {
        Err(Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Claude Deck's pipe is owned by another account",
        ))
    }
}

/// Named mutex `Local\claude-deck-<sid>`; the handle is closed on drop.
pub struct SingleInstance(HANDLE);

// SAFETY: a kernel mutex handle is just an opaque id usable from any thread.
unsafe impl Send for SingleInstance {}
unsafe impl Sync for SingleInstance {}

impl SingleInstance {
    /// `Ok(None)` when another helper already holds the mutex.
    pub fn acquire(sid: &str) -> std::io::Result<Option<Self>> {
        let name: Vec<u16> = format!(r"Local\claude-deck-{sid}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: `name` is NUL-terminated and outlives the call; GetLastError is read
        // immediately after CreateMutexW on the same thread, before anything can clobber it.
        unsafe {
            let h = CreateMutexW(None, false, PCWSTR(name.as_ptr())).map_err(win)?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(h);
                return Ok(None);
            }
            Ok(Some(Self(h)))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and owned by us; closed exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framed::{read_frame, write_frame};
    use deck_proto::{Frame, FrameDecoder};
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);
    fn unique() -> String {
        format!(
            "test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    #[test]
    fn name_and_sid_format() {
        assert_eq!(pipe_name("S-1-5-21-1"), r"\\.\pipe\claude-deck-S-1-5-21-1");
        assert!(current_user_sid().unwrap().starts_with("S-1-5-"));
    }

    #[tokio::test]
    async fn ping_pong_over_pipe() {
        let sid = current_user_sid().unwrap();
        let sec = PipeSecurity::only(&sid).unwrap();
        let name = pipe_name(&unique());
        let mut server = create_server(&name, &sec, true).unwrap();
        let client = tokio::spawn({
            let name = name.clone();
            async move {
                let mut c = connect_client(&name, Duration::from_secs(2)).await.unwrap();
                write_frame(&mut c, &Frame::Control(b"ping".to_vec()))
                    .await
                    .unwrap();
                let mut dec = FrameDecoder::new();
                read_frame(&mut c, &mut dec).await.unwrap()
            }
        });
        server.connect().await.unwrap();
        let mut dec = FrameDecoder::new();
        let got = read_frame(&mut server, &mut dec).await.unwrap().unwrap();
        assert_eq!(got, Frame::Control(b"ping".to_vec()));
        write_frame(&mut server, &Frame::Control(b"pong".to_vec()))
            .await
            .unwrap();
        assert_eq!(
            client.await.unwrap(),
            Some(Frame::Control(b"pong".to_vec()))
        );
    }

    #[tokio::test]
    async fn second_first_instance_fails() {
        let sid = current_user_sid().unwrap();
        let sec = PipeSecurity::only(&sid).unwrap();
        let name = pipe_name(&unique());
        let _first = create_server(&name, &sec, true).unwrap();
        assert!(create_server(&name, &sec, true).is_err());
    }

    #[tokio::test]
    async fn owner_check_and_connect_trusted() {
        let sid = current_user_sid().unwrap();
        let sec = PipeSecurity::only(&sid).unwrap();
        let name = pipe_name(&unique());
        let _server = create_server(&name, &sec, true).unwrap();
        let c = connect_trusted(&name, Duration::from_secs(2), &sid)
            .await
            .unwrap();
        assert!(server_owned_by(&c, &sid).unwrap());
        assert!(server_owned_by(&c, &sid.to_lowercase()).unwrap());
        assert!(!server_owned_by(&c, "S-1-5-18").unwrap());
        let _second = create_server(&name, &sec, false).unwrap(); // free instance for the next client
        let err = connect_trusted(&name, Duration::from_secs(2), "S-1-5-18")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            err.to_string(),
            "Claude Deck's pipe is owned by another account"
        );
    }

    #[test]
    fn single_instance_is_exclusive() {
        let sid = unique();
        let a = SingleInstance::acquire(&sid).unwrap();
        assert!(a.is_some());
        assert!(SingleInstance::acquire(&sid).unwrap().is_none());
        drop(a);
        assert!(SingleInstance::acquire(&sid).unwrap().is_some());
    }
}
