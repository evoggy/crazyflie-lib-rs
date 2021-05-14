mod log;
mod param;

pub use crate::log::Log;
pub use crate::param::Param;

use async_trait::async_trait;
use crazyflie_link::Packet;
use flume as channel;
use flume::{Receiver, Sender};
use std::{collections::HashMap, convert::TryInto, sync::Arc};

pub struct Crazyflie {
    uplink: channel::Sender<Packet>,
    pub log: Log,
    pub param: Param,
}

impl Crazyflie {
    pub async fn connect_from_uri(link_context: &crazyflie_link::LinkContext, uri: &str) -> Self {
        let link = link_context.open_link(uri).await.unwrap();

        Self::connect_from_link(link).await
    }

    pub async fn connect_from_link(link: crazyflie_link::Connection) -> Self {
        // Downlink dispatcher
        let link = Arc::new(link);
        let mut dispatcher = CrtpDispatch::new(link.clone());

        // Uplink queue
        let (uplink, rx) = channel::unbounded();
        async_std::task::spawn(async move {
            loop {
                if let Ok(pk) = rx.recv_async().await {
                    if let Err(_) = link.send_packet(pk).await {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        // Create subsystems one by one
        // The future is passed to join!() later down so that all modules initializes at the same time
        let log_downlink = dispatcher.get_port_receiver(5).unwrap();
        let log = Log::new(log_downlink, uplink.clone());

        let param_downlink = dispatcher.get_port_receiver(2).unwrap();
        let param = Param::new(param_downlink, uplink.clone());

        // Start the downlink packet dispatcher
        dispatcher.run().await;

        // Intitialize all modules in parallel
        let (log, param) = futures::join!(log, param);

        Crazyflie { uplink, log, param }
    }
}

struct CrtpDispatch {
    link: Arc<crazyflie_link::Connection>,
    // port_callbacks: [Arc<Mutex<Option<Sender<Packet>>>>; 15]
    port_channels: HashMap<u8, Sender<Packet>>,
}

impl CrtpDispatch {
    fn new(link: Arc<crazyflie_link::Connection>) -> Self {
        CrtpDispatch {
            link,
            port_channels: HashMap::new(),
        }
    }

    fn get_port_receiver(&mut self, port: u8) -> Option<Receiver<Packet>> {
        if self.port_channels.contains_key(&port) {
            None
        } else {
            let (tx, rx) = channel::unbounded();
            self.port_channels.insert(port, tx);
            Some(rx)
        }
    }

    async fn run(self) {
        let link = self.link.clone();
        async_std::task::spawn(async move {
            loop {
                let packet = link.recv_packet().await.unwrap();
                if packet.get_port() < 16 {
                    let channel = self.port_channels.get(&packet.get_port()); // get(packet.get_port()).lock().await;
                    if let Some(channel) = channel.as_ref() {
                        let _ = channel.send_async(packet).await;
                    }
                }
            }
        });
    }
}

#[async_trait]
trait WaitForPacket {
    async fn wait_packet(&self, port: u8, channel: u8, data_prefix: &[u8]) -> Packet;
}

#[async_trait]
impl WaitForPacket for channel::Receiver<Packet> {
    async fn wait_packet(&self, port: u8, channel: u8, data_prefix: &[u8]) -> Packet {
        let mut pk = self.recv_async().await.unwrap();

        loop {
            if pk.get_port() == port
                && pk.get_channel() == channel
                && pk.get_data().starts_with(data_prefix)
            {
                break;
            }
            pk = self.recv_async().await.unwrap();
        }

        pk
    }
}

const TOC_CHANNEL: u8 = 0;
const TOC_GET_ITEM: u8 = 2;
const TOC_INFO: u8 = 3;

async fn fetch_toc<T: From<u8>>(
    port: u8,
    uplink: channel::Sender<Packet>,
    downlink: channel::Receiver<Packet>,
) -> std::collections::HashMap<String, (u16, T)> {
    println!("Sending log request ...");
    let pk = Packet::new(port, 0, vec![TOC_INFO]);
    uplink.send_async(pk).await.unwrap();

    let pk = downlink.wait_packet(port, TOC_CHANNEL, &[TOC_INFO]).await;

    let toc_len = u16::from_le_bytes(pk.get_data()[1..3].try_into().unwrap());

    println!("Log len: {}", toc_len);

    let mut toc = std::collections::HashMap::new();

    for i in 0..toc_len {
        let pk = Packet::new(
            port,
            0,
            vec![TOC_GET_ITEM, (i & 0x0ff) as u8, (i >> 8) as u8],
        );
        uplink.send_async(pk).await.unwrap();

        let pk = downlink.wait_packet(port, 0, &[TOC_GET_ITEM]).await;

        let mut strings = pk.get_data()[4..].split(|b| *b == 0);
        let group = String::from_utf8_lossy(strings.next().expect("TOC packet format error"));
        let name = String::from_utf8_lossy(strings.next().expect("TOC packet format error"));

        // println!("{}: {}.{}", port, &group, &name);
        let id = u16::from_le_bytes(pk.get_data()[1..3].try_into().unwrap());
        let item_type = pk.get_data()[3].into();
        toc.insert(format!("{}.{}", group, name), (id, item_type));
    }

    toc
}