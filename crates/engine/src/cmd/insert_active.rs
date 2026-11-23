//! Make an insert inactive or active again. An inactive insert keeps its plug-in and settings but
//! is not processed at all (it uses no DSP), unlike bypass, which a plug-in may still run.
use super::{CommandSpec, bad, has_selection};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::INSERT_SLOTS;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "mix.insert_active",
        "Make Insert Active/Inactive",
        [],
        None,
        "{track?, slot, value?: bool} (true = active; toggles when omitted)",
        has_selection,
        insert_active
    )]
}

fn insert_active(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = super::mix::track_slot(e, p, "mix.insert_active", INSERT_SLOTS)?;
    let val = p.get("value").and_then(Value::as_bool);
    let ins = e
        .session_mut()
        .track_mut(t)
        .and_then(|tr| tr.mixer.inserts.get_mut(slot))
        .and_then(Option::as_mut)
        .ok_or_else(|| bad("mix.insert_active", "empty slot"))?;
    ins.active = val.unwrap_or(!ins.active);
    Ok(json!({"active": ins.active}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kick_comp(e: &Engine) -> (soundcraft_model::TrackId, usize) {
        let kick = e.session().track_by_name("Kick").expect("demo has Kick");
        let slot = kick.mixer.inserts.iter().position(|i| i.as_ref().is_some_and(|i| i.plugin == "compressor")).expect("Kick has a compressor");
        (kick.id, slot)
    }

    #[test]
    fn sets_toggles_and_is_reported_by_inspect_and_undoable() {
        let mut e = crate::demo::demo_engine();
        let (kick, slot) = kick_comp(&e);
        let active = |e: &Engine| e.session().track(kick).and_then(|t| t.mixer.inserts.get(slot).cloned().flatten()).map(|i| i.active);
        assert_eq!(active(&e), Some(true));
        let r = e.execute("mix.insert_active", &json!({"track": "Kick", "slot": slot, "value": false})).unwrap();
        assert_eq!(r["active"], false);
        assert_eq!(active(&e), Some(false));
        let doc = crate::inspect::session(&e, false);
        let ins = doc["tracks"].as_array().unwrap().iter().find(|t| t["name"] == "Kick").unwrap()["inserts"][slot].clone();
        assert_eq!(ins["active"], false, "inspect reports the state: {ins}");
        // Omitting the value toggles it back.
        e.execute("mix.insert_active", &json!({"track": "Kick", "slot": slot})).unwrap();
        assert_eq!(active(&e), Some(true));
        e.undo();
        assert_eq!(active(&e), Some(false));
    }

    #[test]
    fn an_empty_or_out_of_range_slot_is_an_error_not_a_crash() {
        let mut e = crate::demo::demo_engine();
        assert!(e.execute("mix.insert_active", &json!({"track": "Snare", "slot": 9, "value": false})).is_err());
        assert!(e.execute("mix.insert_active", &json!({"track": "Kick", "slot": 99})).is_err());
        assert!(e.execute("mix.insert_active", &json!({"track": "Kick", "slot": "z"})).is_err());
    }
}
