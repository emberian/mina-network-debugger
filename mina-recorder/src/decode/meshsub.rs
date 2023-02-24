use std::{io::Cursor, time::SystemTime};

use libp2p_core::PeerId;
use mina_p2p_messages::{
    GossipNetMessageV1,
    gossip::GossipNetMessageV2,
    v2::{
        NetworkPoolSnarkPoolDiffVersionedStableV2, TransactionSnarkWorkStatementStableV2, self,
        TransactionSnarkWorkTStableV2Proofs, MinaBlockBlockStableV2,
    },
};
use binprot::BinProtRead;
use serde::Serialize;
use prost::{bytes::Bytes, Message};

use super::{DecodeError, MessageType, meshsub_stats::Hash, LedgerHash};

#[allow(clippy::derive_partial_eq_without_eq)]
mod pb {
    include!(concat!(env!("OUT_DIR"), "/gossipsub.pb.rs"));
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum Event {
    Subscribe {
        topic: String,
    },
    Unsubscribe {
        topic: String,
    },
    Publish {
        from: Option<String>,
        seqno: Option<String>,
        signature: Option<String>,
        key: Option<String>,
        topic: String,
        message: Box<GossipNetMessageV1>,
    },
    #[serde(rename = "publish_v2")]
    PublishV2 {
        from: Option<PeerId>,
        seqno: Option<String>,
        signature: Option<String>,
        key: Option<String>,
        topic: String,
        message: Box<GossipNetMessageV2>,
        #[serde(skip_serializing)]
        hash: [u8; 32],
    },
    #[serde(rename = "publish")]
    PublishPreview {
        topic: String,
        message: GossipNetMessagePreview,
    },
    PublishTestingMessage {
        from: PeerId,
        topic: String,
        message: String,
        hash: [u8; 32],
    },
    Control {
        ihave: Vec<ControlIHave>,
        iwant: Vec<ControlIWant>,
        graft: Vec<ControlGraft>,
        prune: Vec<ControlPrune>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlIHave {
    topic_id: Option<String>,
    message_ids: Vec<String>,
}

impl ControlIHave {
    pub fn hashes(&self) -> impl Iterator<Item = Hash> + '_ {
        self.message_ids
            .iter()
            .filter_map(|id| Some(Hash(hex::decode(id).ok()?.try_into().ok()?)))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlIWant {
    message_ids: Vec<String>,
}

impl ControlIWant {
    pub fn hashes(&self) -> impl Iterator<Item = Hash> + '_ {
        self.message_ids
            .iter()
            .filter_map(|id| Some(Hash(hex::decode(id).ok()?.try_into().ok()?)))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlGraft {
    topic_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlPrune {
    topic_id: Option<String>,
    peers: Vec<PeerInfo>,
    backoff: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PeerInfo {
    peer_id: Option<String>,
    signed_peer_record: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "message", rename_all = "snake_case")]
pub enum GossipNetMessagePreview {
    NewState,
    SnarkPoolDiff,
    TransactionPoolDiff,
}

#[derive(Serialize)]
pub struct SnarkByHash {
    pub source: Vec<(SnarkWithHash, SystemTime)>,
    pub target: Vec<(SnarkWithHash, SystemTime)>,
    pub first_source: Vec<(SnarkWithHash, SystemTime)>,
    pub middle: Vec<(SnarkWithHash, SystemTime)>,
    pub second_target: Vec<(SnarkWithHash, SystemTime)>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum SnarkWithHash {
    Leaf { hashes: [v2::LedgerHash; 2] },
    Merge { hashes: [v2::LedgerHash; 3] },
}

impl SnarkWithHash {
    pub fn try_from_inner(inner: &NetworkPoolSnarkPoolDiffVersionedStableV2) -> Option<Self> {
        if let NetworkPoolSnarkPoolDiffVersionedStableV2::AddSolvedWork(w) = inner {
            match &w.0 {
                TransactionSnarkWorkStatementStableV2::Two((l, r)) => Some(SnarkWithHash::Merge {
                    hashes: [
                        l.source.ledger.clone(),
                        l.target.ledger.clone(),
                        r.target.ledger.clone(),
                    ],
                }),
                TransactionSnarkWorkStatementStableV2::One(w) => Some(SnarkWithHash::Leaf {
                    hashes: [w.source.ledger.clone(), w.target.ledger.clone()],
                }),
            }
        } else {
            None
        }
    }

    pub fn try_from_block(block: &MinaBlockBlockStableV2) -> Vec<Self> {
        let mut snarks = vec![];
        let it0 = block.body.staged_ledger_diff.diff.0.completed_works.iter();
        let it1 = block
            .body
            .staged_ledger_diff
            .diff
            .1
            .as_ref()
            .into_iter()
            .flat_map(|x| x.completed_works.iter());
        for di in it0.chain(it1) {
            match &di.proofs {
                TransactionSnarkWorkTStableV2Proofs::One(w) => {
                    let source = w.0.statement.source.ledger.clone();
                    let target = w.0.statement.target.ledger.clone();
                    snarks.push(SnarkWithHash::Leaf {
                        hashes: [source, target],
                    })
                }
                TransactionSnarkWorkTStableV2Proofs::Two((f, s)) => {
                    let l = f.0.statement.source.ledger.clone();
                    let m = f.0.statement.target.ledger.clone();
                    let r = s.0.statement.target.ledger.clone();
                    snarks.push(SnarkWithHash::Merge { hashes: [l, m, r] })
                }
            }
        }
        snarks
    }
}

pub fn parse_types(bytes: &[u8]) -> Result<(Vec<MessageType>, Vec<LedgerHash>), DecodeError> {
    let buf = Bytes::from(bytes.to_vec());
    let pb::Rpc {
        subscriptions,
        publish,
        control,
    } = Message::decode_length_delimited(buf).map_err(DecodeError::Protobuf)?;
    let subscriptions = subscriptions.into_iter().map(|v| {
        if v.subscribe() {
            MessageType::Subscribe
        } else {
            MessageType::Unsubscribe
        }
    });
    let mut ledger_hashes = vec![];
    let publish = publish
        .into_iter()
        .filter_map(|msg| msg.data)
        .filter_map(|data| Some((data.get(8).cloned()?, data)))
        .filter_map(|(tag, data)| match tag {
            0 => {
                let mut c = Cursor::new(&data[8..]);
                match GossipNetMessageV2::binprot_read(&mut c) {
                    Ok(GossipNetMessageV2::NewState(block)) => {
                        let it0 = block.body.staged_ledger_diff.diff.0.completed_works.iter();
                        let it1 = block
                            .body
                            .staged_ledger_diff
                            .diff
                            .1
                            .as_ref()
                            .into_iter()
                            .flat_map(|x| x.completed_works.iter());
                        for di in it0.chain(it1) {
                            match &di.proofs {
                                TransactionSnarkWorkTStableV2Proofs::One(w) => {
                                    let source = w.0.statement.source.ledger.clone().into_inner();
                                    let mut h = [0; 31];
                                    h.clone_from_slice(&source.0.as_ref()[1..]);
                                    ledger_hashes.push(LedgerHash::Source(h));
                                    let target = w.0.statement.target.ledger.clone().into_inner();
                                    let mut h = [0; 31];
                                    h.clone_from_slice(&target.0.as_ref()[1..]);
                                    ledger_hashes.push(LedgerHash::Target(h));
                                }
                                TransactionSnarkWorkTStableV2Proofs::Two((f, s)) => {
                                    let l = f.0.statement.source.ledger.clone().into_inner();
                                    let mut h = [0; 31];
                                    h.clone_from_slice(&l.0.as_ref()[1..]);
                                    ledger_hashes.push(LedgerHash::FirstSource(h));
                                    let l = f.0.statement.target.ledger.clone().into_inner();
                                    let mut h = [0; 31];
                                    h.clone_from_slice(&l.0.as_ref()[1..]);
                                    ledger_hashes.push(LedgerHash::Middle(h));
                                    let l = s.0.statement.target.ledger.clone().into_inner();
                                    let mut h = [0; 31];
                                    h.clone_from_slice(&l.0.as_ref()[1..]);
                                    ledger_hashes.push(LedgerHash::SecondTarget(h));
                                }
                            }
                        }
                    }
                    _ => (),
                }
                Some(MessageType::PublishNewState)
            }
            1 => {
                let mut c = Cursor::new(&data[8..]);
                match GossipNetMessageV2::binprot_read(&mut c) {
                    Ok(GossipNetMessageV2::SnarkPoolDiff(
                        NetworkPoolSnarkPoolDiffVersionedStableV2::AddSolvedWork(w),
                    )) => match &w.0 {
                        TransactionSnarkWorkStatementStableV2::One(w) => {
                            let source = w.source.ledger.clone().into_inner();
                            let mut h = [0; 31];
                            h.clone_from_slice(&source.0.as_ref()[1..]);
                            ledger_hashes.push(LedgerHash::Source(h));
                            let target = w.source.ledger.clone().into_inner();
                            let mut h = [0; 31];
                            h.clone_from_slice(&target.0.as_ref()[1..]);
                            ledger_hashes.push(LedgerHash::Target(h));
                        }
                        TransactionSnarkWorkStatementStableV2::Two((f, s)) => {
                            let l = f.source.ledger.clone().into_inner();
                            let mut h = [0; 31];
                            h.clone_from_slice(&l.0.as_ref()[1..]);
                            ledger_hashes.push(LedgerHash::FirstSource(h));
                            let l = f.target.ledger.clone().into_inner();
                            let mut h = [0; 31];
                            h.clone_from_slice(&l.0.as_ref()[1..]);
                            ledger_hashes.push(LedgerHash::Middle(h));
                            let l = s.target.ledger.clone().into_inner();
                            let mut h = [0; 31];
                            h.clone_from_slice(&l.0.as_ref()[1..]);
                            ledger_hashes.push(LedgerHash::SecondTarget(h));
                        }
                    },
                    _ => (),
                }
                Some(MessageType::PublishSnarkPoolDiff)
            }
            2 => Some(MessageType::PublishTransactionPoolDiff),
            _ => None,
        });
    let mut control_types = vec![];
    if let Some(c) = control {
        if !c.ihave.is_empty() {
            control_types.push(MessageType::ControlIHave);
        }
        if !c.iwant.is_empty() {
            control_types.push(MessageType::ControlIWant);
        }
        if !c.graft.is_empty() {
            control_types.push(MessageType::ControlGraft);
        }
        if !c.prune.is_empty() {
            control_types.push(MessageType::ControlPrune);
        }
    }

    let tys = subscriptions.chain(control_types).chain(publish).collect();

    Ok((tys, ledger_hashes))
}

pub fn parse(bytes: Vec<u8>, preview: bool) -> Result<serde_json::Value, DecodeError> {
    let t = parse_it(&bytes, preview, false)?.collect::<Vec<_>>();
    serde_json::to_value(&t).map_err(DecodeError::Serde)
}

pub fn parse_protobuf_publish(
    bytes: &[u8],
) -> Result<impl Iterator<Item = Vec<u8>>, prost::DecodeError> {
    let pb::Rpc { publish, .. } = Message::decode_length_delimited(bytes)?;

    Ok(publish.into_iter().filter_map(|m| m.data))
}

pub fn parse_it(
    bytes: &[u8],
    preview: bool,
    calc_hash: bool,
) -> Result<impl Iterator<Item = Event>, DecodeError> {
    let pb::Rpc {
        subscriptions,
        publish,
        control,
    } = Message::decode_length_delimited(bytes).map_err(DecodeError::Protobuf)?;
    let subscriptions = subscriptions.into_iter().map(|v| {
        let subscribe = v.subscribe();
        let topic = v.topic_id.unwrap_or_default();
        if subscribe {
            Event::Subscribe { topic }
        } else {
            Event::Unsubscribe { topic }
        }
    });
    let publish = publish
        .into_iter()
        .filter_map(|msg| {
            Some((
                msg.data?,
                msg.topic,
                msg.from,
                msg.seqno,
                msg.signature,
                msg.key,
            ))
        })
        .filter_map(move |(data, topic, from, seqno, signature, key)| {
            let mut c = Cursor::new(&data[8..]);
            match GossipNetMessageV2::binprot_read(&mut c) {
                Ok(msg) => {
                    let message = Box::new(msg);
                    if preview {
                        let message = match &*message {
                            GossipNetMessageV2::NewState(_) => GossipNetMessagePreview::NewState,
                            GossipNetMessageV2::SnarkPoolDiff(_) => {
                                GossipNetMessagePreview::SnarkPoolDiff
                            }
                            GossipNetMessageV2::TransactionPoolDiff(_) => {
                                GossipNetMessagePreview::TransactionPoolDiff
                            }
                        };
                        return Some(Event::PublishPreview { topic, message });
                    } else {
                        let hash = if calc_hash {
                            use blake2::digest::{Mac, Update, FixedOutput, typenum};

                            let key;
                            let key = if topic.as_bytes().len() <= 64 {
                                topic.as_bytes()
                            } else {
                                key = blake2::Blake2b::<typenum::U32>::default()
                                    .chain(topic.as_bytes())
                                    .finalize_fixed();
                                key.as_slice()
                            };
                            blake2::Blake2bMac::<typenum::U32>::new_from_slice(key)
                                .expect("cannot fail, length is statically known")
                                .chain(data)
                                .finalize_fixed()
                                .into()
                        } else {
                            [0; 32]
                        };
                        return Some(Event::PublishV2 {
                            from: from.and_then(|b| PeerId::from_bytes(&b).ok()),
                            seqno: seqno.map(hex::encode),
                            signature: signature.map(hex::encode),
                            key: key.map(hex::encode),
                            topic,
                            message,
                            hash,
                        });
                    }
                }
                Err(err) => log::error!("decode {err}"),
            }

            let mut c = Cursor::new(&data[8..]);

            if let Some(3) = c.get_ref().first() {
                let bytes = c.get_ref()[1..].to_vec();
                let message = String::from_utf8(bytes).ok()?;
                let from = PeerId::from_bytes(&from?).ok()?;

                let hash = if calc_hash {
                    use blake2::digest::{Mac, Update, FixedOutput, typenum};

                    let key;
                    let key = if topic.as_bytes().len() <= 64 {
                        topic.as_bytes()
                    } else {
                        key = blake2::Blake2b::<typenum::U32>::default()
                            .chain(topic.as_bytes())
                            .finalize_fixed();
                        key.as_slice()
                    };
                    blake2::Blake2bMac::<typenum::U32>::new_from_slice(key)
                        .expect("cannot fail, length is statically known")
                        .chain(data)
                        .finalize_fixed()
                        .into()
                } else {
                    [0; 32]
                };

                return Some(Event::PublishTestingMessage { from, topic, message, hash });
            }

            match GossipNetMessageV1::binprot_read(&mut c) {
                Ok(msg) => {
                    let message = Box::new(msg);
                    if preview {
                        let message = match &*message {
                            GossipNetMessageV1::NewState(_) => GossipNetMessagePreview::NewState,
                            GossipNetMessageV1::SnarkPoolDiff(_) => {
                                GossipNetMessagePreview::SnarkPoolDiff
                            }
                            GossipNetMessageV1::TransactionPoolDiff(_) => {
                                GossipNetMessagePreview::TransactionPoolDiff
                            }
                        };
                        return Some(Event::PublishPreview { topic, message });
                    } else {
                        return Some(Event::Publish {
                            from: from.map(hex::encode),
                            seqno: seqno.map(hex::encode),
                            signature: signature.map(hex::encode),
                            key: key.map(hex::encode),
                            topic,
                            message,
                        });
                    }
                }
                Err(err) => log::error!("decode {err}"),
            }

            None
        });
    let control = control.into_iter().map(
        |pb::ControlMessage {
             ihave,
             iwant,
             graft,
             prune,
         }| Event::Control {
            ihave: ihave
                .into_iter()
                .map(|m| ControlIHave {
                    topic_id: m.topic_id,
                    message_ids: m.message_ids.into_iter().map(hex::encode).collect(),
                })
                .collect(),
            iwant: iwant
                .into_iter()
                .map(|m| ControlIWant {
                    message_ids: m.message_ids.into_iter().map(hex::encode).collect(),
                })
                .collect(),
            graft: graft
                .into_iter()
                .map(|m| ControlGraft {
                    topic_id: m.topic_id,
                })
                .collect(),
            prune: prune
                .into_iter()
                .map(|m| ControlPrune {
                    topic_id: m.topic_id,
                    peers: m
                        .peers
                        .into_iter()
                        .map(|peer| PeerInfo {
                            peer_id: peer.peer_id.map(hex::encode),
                            signed_peer_record: peer.signed_peer_record.map(hex::encode),
                        })
                        .collect(),
                    backoff: m.backoff,
                })
                .collect(),
        },
    );

    Ok(subscriptions.chain(publish).chain(control))
}

#[cfg(test)]
#[test]
fn tag0_msg() {
    use std::io::Cursor;

    use mina_p2p_messages::v1::MinaBlockExternalTransitionRawVersionedStableV1Versioned as Msg;

    use prost::{bytes::Bytes, Message as _};

    let buf = Bytes::from(hex::decode(include_str!("tag_0.hex")).expect("test"));
    let msg = pb::Rpc::decode_length_delimited(buf).expect("test");
    for a in msg
        .publish
        .into_iter()
        .filter_map(|msg| msg.data)
        .map(|data| {
            if data[8] == 0 {
                let mut c = Cursor::new(&data[9..]);
                Some(Msg::binprot_read(&mut c).expect("test"))
            } else {
                None
            }
        })
    {
        println!("{a:?}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_new_berkeley_2() {
        use std::io::Cursor;
        use binprot::BinProtRead;
        use mina_p2p_messages::gossip::GossipNetMessageV2;

        let hex_str = include_str!("test_data_2.hex");
        let data = hex::decode(hex_str).unwrap();
        let mut c = Cursor::new(data);
        GossipNetMessageV2::binprot_read(&mut c).unwrap();
    }

    #[test]
    fn parse_new_berkeley_3() {
        use std::io::Cursor;
        use binprot::BinProtRead;
        use mina_p2p_messages::gossip::GossipNetMessageV2;

        let hex_str = include_str!("test_data_3.hex");
        let data = hex::decode(hex_str).unwrap();
        let mut c = Cursor::new(data);
        GossipNetMessageV2::binprot_read(&mut c).unwrap();
    }
}
