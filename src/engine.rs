use std::fmt::{self, Write};
use std::str::FromStr;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time;

use crate::timing;
use crate::storage::get_storage;
use crate::config::PhaseQueenConfig;
use crate::state::PhaseQueenState;
use crate::node::PhaseQueenNode;

use sawtooth_sdk::consensus::{engine::*, service::Service};

pub struct PhaseQueenEngine {
    config: PhaseQueenConfig,
}

impl PhaseQueenEngine {
    pub fn new(config: PhaseQueenConfig) -> Self {
        PhaseQueenEngine { config }
    }
}

impl Engine for PhaseQueenEngine {
    #[allow(clippy::cognitive_complexity)]
    fn start(
        &mut self,
        updates: Receiver<Update>,
        mut service: Box<dyn Service>,
        startup_state: StartupState,
    ) -> Result<(), Error> {
        info!("Startup state received from validator: {:?}", startup_state);

        let StartupState {
            chain_head,
            peers,
            local_peer_info,
        } = startup_state;

        // Load on-chain settings
        self.config
            .load_settings(chain_head.block_id.clone(), &mut *service);

        info!("PhaseQueen config loaded: {:?}", self.config);

        let mut phase_queen_state = get_storage(&self.config.storage_location, || {
            PhaseQueenState::new(
                local_peer_info.peer_id.clone(),
                chain_head.block_num,
                &self.config,
            )
        })
        .unwrap_or_else(|err| panic!("Failed to load state due to error: {}", err));

        info!("PhaseQueenState state created: {}", **phase_queen_state.read());

        let mut block_publishing_ticker = timing::Ticker::new(self.config.block_publishing_delay);

        let mut node = PhaseQueenNode::new(
            &self.config,
            chain_head,
            peers,
            service,
            &mut phase_queen_state.write(),
        );

        // TODO: debug, rimuovere poi
        let mut timestamp_log = time::Instant::now();

        loop {
            let incoming_message = updates.recv_timeout(time::Duration::from_millis(10));
            let state = &mut **phase_queen_state.write();

            match handle_update(&mut node, incoming_message, state) {
                Ok(again) => {
                    if !again {
                        break;
                    }
                }
                Err(err) => error!("{}", err),
            }

            block_publishing_ticker.tick(|| node.try_publish(state));

            if time::Instant::now().duration_since(timestamp_log) > time::Duration::from_secs(5) {
                info!("My state: {}", state);
                timestamp_log = time::Instant::now();
            }
        }

        Ok(())
    }

    fn version(&self) -> String {
        "0.1".into()
    }

    fn name(&self) -> String {
        "Devmode".into()
    }

    fn additional_protocols(&self) -> Vec<(String, String)> {
        vec![]
    }
}

struct DisplayBlock<'b>(&'b Block);

impl<'b> fmt::Display for DisplayBlock<'b> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Block(")?;
        f.write_str(&self.0.block_num.to_string())?;
        write!(f, ", id: {}", to_hex(&self.0.block_id))?;
        write!(f, ", prev: {})", to_hex(&self.0.previous_id))
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut buf = String::new();
    for b in bytes {
        write!(&mut buf, "{:0x}", b).expect("Unable to write to string");
    }

    buf
}

pub enum PhaseQueenMessage {
    Exchange,
    QueenExchange,
}

impl FromStr for PhaseQueenMessage {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "exchange" => Ok(PhaseQueenMessage::Exchange),
            "queen_exchange" => Ok(PhaseQueenMessage::QueenExchange),
            _ => Err("Invalid message type"),
        }
    }
}

fn handle_update(
    node: &mut PhaseQueenNode,
    incoming_message: Result<Update, RecvTimeoutError>,
    state: &mut PhaseQueenState,
) -> Result<bool, Error> {
    match incoming_message {
        Ok(Update::BlockNew(block)) => node.on_block_new(block, state),
        Ok(Update::BlockValid(block_id)) => node.on_block_valid(block_id, state),
        Ok(Update::BlockInvalid(block_id)) => node.on_block_invalid(block_id),
        Ok(Update::BlockCommit(block_id)) => node.on_block_commit(block_id, state),
        Ok(Update::PeerMessage(message, _)) => {
            node.on_peer_message(message.header.message_type.as_ref(), *first(&message.content).unwrap(), state);
            return Ok(true);
        }
        Ok(Update::Shutdown) => {
            info!("Received shutdown; stopping PBFT");
            return Ok(false);
        }
        Ok(Update::PeerConnected(info)) => {
            node.on_peer_connected(info.peer_id, state);
            return Ok(true);
        }
        Ok(Update::PeerDisconnected(id)) => {
            info!("Received PeerDisconnected for peer ID: {:?}", id);
            return Ok(false);
        }
        Err(RecvTimeoutError::Timeout) => { return Ok(true); },
        Err(RecvTimeoutError::Disconnected) => {
            error!("Disconnected from validator; stopping PhaseQueen");
            return Ok(false);
        }
    };

    Ok(true)
}

// https://stackoverflow.com/questions/36876570/return-first-item-of-vector
fn first<T>(v: &Vec<T>) -> Option<&T> {
    v.first()
}