use super::Error;
use crate::cli::Local;
use futures::SinkExt;
use futures::StreamExt;
use std::net::Ipv4Addr;
use tokio;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::net::UnixListener;
use tokio::net::UnixStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

pub enum Listener {
    Tcp(TcpListener),
    Unix(UnixListener),
}

impl Listener {
    pub(crate) async fn bind(local: Local) -> Result<Self, Error> {
        Ok(match local {
            Local::Port(port) => {
                let listener = TcpListener::bind((Ipv4Addr::from([127, 0, 0, 1]), port)).await?;
                Listener::Tcp(listener)
            }
            Local::Unix(path) => {
                let listener = UnixListener::bind(path)?;
                Listener::Unix(listener)
            }
        })
    }

    pub(crate) async fn accept(&self) -> Result<RemoteConnection, Error> {
        match self {
            Listener::Tcp(listener) => {
                let (socket, _addr) = listener.accept().await?;
                println!("new connection with addr {:?}", _addr);
                Ok(RemoteConnection::Tcp(socket))
            }
            Listener::Unix(listener) => {
                let (socket, _addr) = listener.accept().await?;
                Ok(RemoteConnection::Unix(socket))
            }
        }
    }
}

pub(crate) enum RemoteConnection {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl RemoteConnection {
    pub(crate) async fn copy(self, url: Url) -> std::result::Result<(), Error> {
        match self {
            RemoteConnection::Tcp(tcp_stream) => copy(tcp_stream, url).await,
            RemoteConnection::Unix(unix_stream) => copy(unix_stream, url).await,
        }
    }
}

// proxy local connection to websocket
pub(crate) async fn copy<T>(socket: T, url: Url) -> std::result::Result<(), Error>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (mut writer, mut reader) = (ws_stream).split();
    let (mut socket_reader, mut socket_writer) = tokio::io::split(socket);
    let mut buf = [0; 4096];
    tokio::select! {
        r = async {
            loop{
                match socket_reader.read(&mut buf).await? {
                    0 => {
                        break;
                    }
                    n => {
                        writer.send(Message::binary(&buf[..n])).await?;
                    }
                };
            }
            Ok(())
        }=>r,
        r = async {
            loop{
                let message = reader.next().await;
                if let Some(data) = message {
                    let binary_data = data?;
                    let data = binary_data.into_data();
                    tokio::io::AsyncWriteExt::write_all(&mut socket_writer, &data).await?;
                } else {
                    break;
                }
            }
            Ok(())
        } => r
    }
}
