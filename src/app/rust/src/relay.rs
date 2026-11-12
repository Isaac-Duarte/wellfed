//! A loopback SOCKS5 relay that takes the owner's server around a Wi-Fi
//! with no way out.
//!
//! On Android the process is bound to the Wi-Fi network, so the device API
//! sessions reach the feeders there while Android keeps mobile data as the
//! default. On a Wi-Fi with no validated internet (a hotspot with no
//! uplink, a captive portal) that binding also holds a server reached by a
//! public name out of reach, though mobile data is up. Kotlin watches the
//! default network and hands its handle over JNI ([`set_uplink`]) only
//! while the bound Wi-Fi has no validated internet and another network
//! has; zero otherwise.
//!
//! reqwest opens its own sockets with no hook to bind them, so the server
//! client takes a proxy ([`proxy`]) that sends a request through this relay
//! only while there is such a network and the server is not a home-network
//! address. The relay opens each connection, and resolves each name, on
//! that network (`android_setsocknetwork`, `android_getaddrinfofornetwork`).
//! A server at home and every feeder session never pass through it: the
//! feeders' sessions are `wellfed-connect`'s own sockets, which take no
//! proxy. On a desktop nothing sets the handle, so nothing goes through
//! it, and binding a socket to a network does nothing.
//!
//! Every app on the phone shares the loopback, so the relay takes only a
//! client that names its per-process user and password (RFC 1929), and
//! only `CONNECT`. It starts on its own thread the first time a request
//! needs it, and lives as long as the process.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use wellfed_api::at_home;
use wellfed_server_client::reqwest::Proxy;

/// The handle of the network the relay goes out on; zero for none.
static UPLINK: AtomicU64 = AtomicU64::new(0);

/// The relay, once started; `None` when it could not start.
static RELAY: OnceLock<Option<Relay>> = OnceLock::new();

/// The longest a client may take to open its tunnel, and the relay to
/// connect it.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// The network to reach a server by while the bound Wi-Fi has no way out:
/// an Android `net_handle_t`, or zero for none.
#[cfg_attr(
    not(any(target_os = "android", test)),
    expect(dead_code, reason = "only Android's JNI hook and the tests set it")
)]
pub(crate) fn set_uplink(handle: u64) {
    UPLINK.store(handle, Ordering::Relaxed);
}

fn uplink() -> Option<u64> {
    let handle = UPLINK.load(Ordering::Relaxed);
    (handle != 0).then_some(handle)
}

/// Whether a request to `host` goes through the relay now.
fn relayed(host: &str) -> bool {
    uplink().is_some() && !at_home(host)
}

/// The server client's proxy: the relay, for a request it should carry
/// ([`relayed`]), and no proxy for any other.
pub(crate) fn proxy() -> Proxy {
    Proxy::custom(|url| {
        if !relayed(url.host_str()?) {
            return None;
        }
        RELAY
            .get_or_init(|| match Relay::start() {
                Ok(relay) => Some(relay),
                Err(error) => {
                    log::warn!("the relay around a Wi-Fi with no way out did not start: {error}");
                    None
                }
            })
            .as_ref()
            .map(Relay::url)
    })
}

/// The running relay: where it listens, and what a client names to use it.
#[derive(Clone, Debug)]
pub(crate) struct Relay {
    address: SocketAddr,
    user: String,
    password: String,
}

impl Relay {
    /// Start a relay on a loopback port, on its own thread.
    pub(crate) fn start() -> io::Result<Self> {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let relay = Self {
            address: listener.local_addr()?,
            user: uuid::Uuid::new_v4().simple().to_string(),
            password: uuid::Uuid::new_v4().simple().to_string(),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let serving = relay.clone();
        std::thread::Builder::new()
            .name("server-relay".into())
            .spawn(move || {
                runtime.block_on(async move {
                    match TcpListener::from_std(listener) {
                        Ok(listener) => serving.serve(listener).await,
                        Err(error) => log::warn!("the relay stopped: {error}"),
                    }
                });
            })?;
        Ok(relay)
    }

    /// The proxy URL a client reaches it by: names resolve at the relay.
    pub(crate) fn url(&self) -> String {
        format!("socks5h://{}:{}@{}", self.user, self.password, self.address)
    }

    async fn serve(self, listener: TcpListener) {
        loop {
            let client = match listener.accept().await {
                Ok((client, _)) => client,
                Err(error) => {
                    // Out of file descriptors, say: the listener holds.
                    log::warn!("the relay missed a connection: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let relay = self.clone();
            tokio::spawn(async move {
                if let Err(error) = relay.carry(client).await {
                    log::debug!("a relayed connection ended: {error}");
                }
            });
        }
    }

    /// One client's tunnel: its handshake, the connection out, and the
    /// bytes both ways until either side closes.
    async fn carry(&self, mut client: TcpStream) -> io::Result<()> {
        let opened = tokio::time::timeout(HANDSHAKE, self.open(&mut client)).await;
        let mut out = opened.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        tokio::io::copy_bidirectional(&mut client, &mut out).await?;
        Ok(())
    }

    /// The SOCKS5 handshake (RFC 1928) with the user and password
    /// (RFC 1929), then the connection to the target on the uplink.
    async fn open(&self, client: &mut TcpStream) -> io::Result<TcpStream> {
        let [version, count] = read_array(client).await?;
        let mut methods = vec![0; usize::from(count)];
        client.read_exact(&mut methods).await?;
        if version != 5 || !methods.contains(&USER_PASSWORD) {
            client.write_all(&[5, NO_METHOD]).await?;
            return Err(refused("no user and password offered"));
        }
        client.write_all(&[5, USER_PASSWORD]).await?;

        let [_, user_len] = read_array(client).await?;
        let user = read_vec(client, user_len).await?;
        let [password_len] = read_array(client).await?;
        let password = read_vec(client, password_len).await?;
        if user != self.user.as_bytes() || password != self.password.as_bytes() {
            client.write_all(&[1, 1]).await?;
            return Err(refused("a wrong user or password"));
        }
        client.write_all(&[1, 0]).await?;

        let [version, command, _, kind] = read_array(client).await?;
        let target = read_target(client, kind).await?;
        if version != 5 || command != CONNECT {
            reply(client, Reply::CommandNotSupported).await?;
            return Err(refused("only CONNECT"));
        }
        let Some(network) = uplink() else {
            reply(client, Reply::NetworkUnreachable).await?;
            return Err(refused("no network to go out on"));
        };
        match connect(network, &target).await {
            Ok(out) => {
                reply(client, Reply::Succeeded).await?;
                Ok(out)
            }
            Err(error) => {
                let answer = match error.kind() {
                    io::ErrorKind::ConnectionRefused => Reply::ConnectionRefused,
                    _ => Reply::HostUnreachable,
                };
                reply(client, answer).await?;
                Err(error)
            }
        }
    }
}

const NO_METHOD: u8 = 0xff;
const USER_PASSWORD: u8 = 0x02;
const CONNECT: u8 = 0x01;

/// Where a client asked to go.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Address(SocketAddr),
    /// A name, resolved on the uplink.
    Name(String, u16),
}

/// A request's answer (RFC 1928, section 6).
#[derive(Clone, Copy)]
enum Reply {
    Succeeded = 0,
    NetworkUnreachable = 3,
    HostUnreachable = 4,
    ConnectionRefused = 5,
    CommandNotSupported = 7,
}

async fn reply(client: &mut TcpStream, reply: Reply) -> io::Result<()> {
    // The bound address is the relay's own business: all zeros.
    client
        .write_all(&[5, reply as u8, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
}

fn refused(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, why.to_owned())
}

async fn read_array<const N: usize>(client: &mut TcpStream) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    client.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn read_vec(client: &mut TcpStream, len: u8) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::from(len)];
    client.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn read_target(client: &mut TcpStream, kind: u8) -> io::Result<Target> {
    let target = match kind {
        1 => {
            let ip = Ipv4Addr::from(read_array::<4>(client).await?);
            Target::Address(SocketAddr::new(IpAddr::V4(ip), 0))
        }
        3 => {
            let [len] = read_array(client).await?;
            let name = String::from_utf8(read_vec(client, len).await?)
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            Target::Name(name, 0)
        }
        4 => {
            let ip = Ipv6Addr::from(read_array::<16>(client).await?);
            Target::Address(SocketAddr::new(IpAddr::V6(ip), 0))
        }
        _ => return Err(io::Error::from(io::ErrorKind::InvalidData)),
    };
    let port = u16::from_be_bytes(read_array(client).await?);
    Ok(match target {
        Target::Address(address) => Target::Address(SocketAddr::new(address.ip(), port)),
        Target::Name(name, _) => Target::Name(name, port),
    })
}

/// A connection to `target` on `network`: each address its name resolves
/// to there, in turn, until one answers.
async fn connect(network: u64, target: &Target) -> io::Result<TcpStream> {
    let addresses = match target {
        Target::Address(address) => vec![*address],
        Target::Name(name, port) => uplink_net::resolve(network, name, *port).await?,
    };

    let mut last = io::Error::from(io::ErrorKind::NotFound);
    for address in addresses {
        let socket = if address.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        uplink_net::bind(network, &socket)?;
        match tokio::time::timeout(HANDSHAKE, socket.connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = error,
            Err(_) => last = io::Error::from(io::ErrorKind::TimedOut),
        }
    }
    Err(last)
}

/// Going out on a network other than the one the process is bound to.
#[cfg(target_os = "android")]
mod uplink_net {
    use std::ffi::{CString, c_char, c_int};
    use std::io;
    use std::net::SocketAddr;
    use std::os::fd::AsRawFd;

    use tokio::net::TcpSocket;

    #[link(name = "android")]
    unsafe extern "C" {
        // `<android/multinetwork.h>`, API 23 and later.
        fn android_setsocknetwork(network: u64, fd: c_int) -> c_int;
        fn android_getaddrinfofornetwork(
            network: u64,
            node: *const c_char,
            service: *const c_char,
            hints: *const libc::addrinfo,
            res: *mut *mut libc::addrinfo,
        ) -> c_int;
    }

    /// Bind `socket` to `network` before it connects.
    pub(super) fn bind(network: u64, socket: &TcpSocket) -> io::Result<()> {
        // SAFETY: the descriptor is the open socket `socket` owns for the
        // whole call, and the call only sets which network it routes over.
        let bound = unsafe { android_setsocknetwork(network, socket.as_raw_fd()) };
        if bound == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// The addresses `name` resolves to on `network`'s own DNS.
    pub(super) async fn resolve(
        network: u64,
        name: &str,
        port: u16,
    ) -> io::Result<Vec<SocketAddr>> {
        let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        tokio::task::spawn_blocking(move || lookup(network, &name, port))
            .await
            .map_err(io::Error::other)?
    }

    fn lookup(network: u64, name: &CString, port: u16) -> io::Result<Vec<SocketAddr>> {
        // SAFETY: an all-zero `addrinfo` is the documented empty hint, and
        // only the stream socket type is set on it.
        let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
        hints.ai_socktype = libc::SOCK_STREAM;
        let mut found: *mut libc::addrinfo = std::ptr::null_mut();
        // SAFETY: `name` is a NUL-terminated string that outlives the call,
        // the service is absent, `hints` is a valid `addrinfo`, and `found`
        // is where the call writes the list it allocates.
        let status = unsafe {
            android_getaddrinfofornetwork(
                network,
                name.as_ptr(),
                std::ptr::null(),
                &hints,
                &mut found,
            )
        };
        if status != 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("getaddrinfo: {status}"),
            ));
        }

        let mut addresses = Vec::new();
        let mut next = found;
        while !next.is_null() {
            // SAFETY: `next` is a node of the list the call returned, which
            // stays allocated until `freeaddrinfo` below.
            let entry = unsafe { &*next };
            if let Some(address) = socket_address(entry, port) {
                addresses.push(address);
            }
            next = entry.ai_next;
        }
        // SAFETY: `found` is the list the call allocated, freed once, and
        // nothing read from it is kept past here but copied addresses.
        unsafe { libc::freeaddrinfo(found) };
        Ok(addresses)
    }

    fn socket_address(entry: &libc::addrinfo, port: u16) -> Option<SocketAddr> {
        if entry.ai_addr.is_null() {
            return None;
        }
        match entry.ai_family {
            libc::AF_INET => {
                // SAFETY: an `AF_INET` entry's address is a `sockaddr_in`.
                let v4 = unsafe { &*entry.ai_addr.cast::<libc::sockaddr_in>() };
                let ip = std::net::Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr));
                Some(SocketAddr::new(ip.into(), port))
            }
            libc::AF_INET6 => {
                // SAFETY: an `AF_INET6` entry's address is a `sockaddr_in6`.
                let v6 = unsafe { &*entry.ai_addr.cast::<libc::sockaddr_in6>() };
                let ip = std::net::Ipv6Addr::from(v6.sin6_addr.s6_addr);
                Some(SocketAddr::new(ip.into(), port))
            }
            _ => None,
        }
    }
}

/// Off Android there is no network to bind a socket to: the relay goes out
/// the way any socket does, which is what its tests use.
#[cfg(not(target_os = "android"))]
mod uplink_net {
    use std::io;
    use std::net::SocketAddr;

    use tokio::net::TcpSocket;

    pub(super) fn bind(_network: u64, _socket: &TcpSocket) -> io::Result<()> {
        Ok(())
    }

    pub(super) async fn resolve(
        _network: u64,
        name: &str,
        port: u16,
    ) -> io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((name, port)).await?.collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_server_off_the_home_network_goes_through_it_and_only_with_an_uplink() {
        set_uplink(0);
        assert!(!relayed("feeder.example.org"), "no uplink, no relay");

        set_uplink(7);
        assert!(relayed("feeder.example.org"));
        assert!(relayed("203.0.113.4"));
        for home in [
            "192.168.1.20",
            "wellfed.local",
            "10.0.0.4",
            "127.0.0.1",
            "[fd00::4]",
        ] {
            assert!(!relayed(home), "{home}");
        }
        set_uplink(0);
    }

    /// A client of the relay naming `url` as its proxy.
    fn through(url: &str) -> wellfed_server_client::reqwest::Client {
        wellfed_server_client::reqwest::Client::builder()
            .proxy(Proxy::all(url).unwrap())
            .build()
            .unwrap()
    }

    #[test]
    fn a_request_through_the_relay_reaches_the_server_on_the_uplink() {
        wellfed_server_testkit::local(async {
            let server = wellfed_server_testkit::TestServer::start().await;
            let relay = Relay::start().unwrap();
            set_uplink(7);

            let url = format!("http://localhost:{}/health", server.address().port());
            let answer = through(&relay.url()).get(url).send().await.unwrap();
            assert!(answer.status().is_success());

            server.stop().await;
            set_uplink(0);
        });
    }

    #[test]
    fn the_relay_takes_only_its_own_user_and_password_and_only_with_an_uplink() {
        wellfed_server_testkit::local(async {
            let server = wellfed_server_testkit::TestServer::start().await;
            let relay = Relay::start().unwrap();
            let url = format!("http://{}/health", server.address());

            set_uplink(7);
            let stranger = format!("socks5h://someone:else@{}", relay.address);
            assert!(through(&stranger).get(&url).send().await.is_err());

            set_uplink(0);
            assert!(
                through(&relay.url()).get(&url).send().await.is_err(),
                "no uplink"
            );

            server.stop().await;
        });
    }
}
