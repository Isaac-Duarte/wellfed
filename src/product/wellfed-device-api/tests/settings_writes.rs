//! A settings write is answered by what the writer did with it: `Done`
//! with the new generation once it saved the change whole, and refused
//! `NotWritable` when it saved none of it or only part, never `Done`.

mod support;

use support::Rig;
use wellfed_proto::api::{
    ClientMessage, DeviceMessage, FRAME_MAX, FeederOp, Op, RequestId, SettingsChanges,
};
use wellfed_session::noise::Transport;

/// Seal `op` under `id` to the slot.
fn ask(rig: &mut Rig, slot: usize, transport: &mut Transport, id: RequestId, op: Op) {
    let mut plain = [0; 256];
    let len = wellfed_proto::encode(&ClientMessage::Request { id, op }, &mut plain).unwrap();
    let mut sealed = [0; FRAME_MAX];
    let len = transport.seal(&plain[..len], &mut sealed).unwrap();
    let bytes = support::framed(&sealed[..len]);
    assert_eq!(rig.deliver(slot, &bytes), bytes.len());
}

/// The answer to `id` once the loop has passed until it is sent.
fn answer(rig: &mut Rig, slot: usize, transport: &mut Transport, id: RequestId) -> String {
    let mut plain = [0; FRAME_MAX];
    for _ in 0..8 {
        rig.pass();
        let found = rig.collect(slot).iter().find_map(|reply| {
            let read = transport.open(reply, &mut plain).unwrap();
            match wellfed_proto::decode::<DeviceMessage<'_>>(&plain[..read]) {
                Ok(DeviceMessage::Response { id: to, result }) if to == id => {
                    Some(format!("{result:?}"))
                }
                _ => None,
            }
        });
        if let Some(found) = found {
            return found;
        }
    }
    panic!("no answer to {id:?}");
}

fn volume(volume: u8) -> SettingsChanges {
    SettingsChanges {
        volume: Some(volume),
        ..SettingsChanges::default()
    }
}

#[test]
fn a_settings_write_the_writer_failed_is_refused_not_done() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();

    let put = |base, changes| Op::Feeder(FeederOp::PutSettings { base, changes });
    ask(&mut rig, slot, &mut transport, [1; 16], put(0, volume(10)));
    let done = answer(&mut rig, slot, &mut transport, [1; 16]);
    assert!(done.starts_with("Done"), "{done}");

    rig.fail_settings(true);
    ask(&mut rig, slot, &mut transport, [2; 16], put(1, volume(20)));
    let failed = answer(&mut rig, slot, &mut transport, [2; 16]);
    assert!(failed.contains("NotWritable"), "{failed}");
}

#[test]
fn a_desiccant_reset_the_writer_failed_is_refused_not_done() {
    let mut rig = Rig::new();
    let (slot, mut transport) = rig.open();

    let reset = Op::Feeder(FeederOp::ResetDesiccant);
    ask(&mut rig, slot, &mut transport, [1; 16], reset);
    let done = answer(&mut rig, slot, &mut transport, [1; 16]);
    assert!(done.starts_with("Done"), "{done}");

    rig.fail_settings(true);
    ask(&mut rig, slot, &mut transport, [2; 16], reset);
    let failed = answer(&mut rig, slot, &mut transport, [2; 16]);
    assert!(failed.contains("NotWritable"), "{failed}");
}
