//! Decode block events into `RawEvent`s.

use std::collections::HashMap;

use tracing::trace;

use maestro_core::metrics::record_decode_error;
use maestro_core::ports::RawEvent;
use subxt::PolkadotConfig;
use subxt::dynamic::Value;

use crate::scale_json::value_to_json;

pub(crate) fn decode_events(events: &subxt::events::Events<PolkadotConfig>) -> Vec<RawEvent> {
    let mut raw_events = Vec::new();

    for (index, event) in events.iter().enumerate() {
        match event {
            Ok(ev) => {
                let pallet = ev.pallet_name().to_string();
                let name = ev.event_name().to_string();

                let data = ev
                    .decode_fields_unchecked_as::<Value>()
                    .map(|value| value_to_json(&value))
                    .unwrap_or_else(|e| {
                        trace!(
                            index,
                            pallet = %pallet,
                            name = %name,
                            error = ?e,
                            "Failed to decode event fields"
                        );
                        record_decode_error("event", &pallet);
                        serde_json::Value::Null
                    });

                let extrinsic_index = match ev.phase() {
                    subxt::events::Phase::ApplyExtrinsic(idx) => Some(idx),
                    _ => None,
                };

                raw_events.push(RawEvent {
                    index: index as u32,
                    extrinsic_index,
                    pallet,
                    name,
                    data,
                    topics: Vec::new(),
                });
            }
            Err(e) => {
                trace!(index, error = ?e, "Failed to decode event");
                record_decode_error("event", "unknown");
            }
        }
    }

    raw_events
}

/// Index `System::ExtrinsicSuccess` / `System::ExtrinsicFailed` events by their
/// `Phase::ApplyExtrinsic(idx)` in a single pass, so per-extrinsic lookups are
/// O(1) instead of O(M). Extrinsics with no result event default to `(true, None)`.
pub(crate) fn build_extrinsic_results(
    events: &subxt::events::Events<PolkadotConfig>,
) -> HashMap<u32, (bool, Option<String>)> {
    let mut map = HashMap::new();
    for ev in events.iter().flatten() {
        let subxt::events::Phase::ApplyExtrinsic(idx) = ev.phase() else {
            continue;
        };
        if ev.pallet_name() != "System" {
            continue;
        }
        match ev.event_name() {
            "ExtrinsicSuccess" => {
                map.insert(idx, (true, None));
            }
            "ExtrinsicFailed" => {
                let error_info = ev
                    .decode_fields_unchecked_as::<Value>()
                    .map(|v| format!("{:?}", v))
                    .unwrap_or_else(|_| "Unknown error".to_string());
                map.insert(idx, (false, Some(error_info)));
            }
            _ => {}
        }
    }
    map
}
