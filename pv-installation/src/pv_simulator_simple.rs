use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use eyre::eyre;
use s2energy::common::{
    Commodity, CommodityQuantity, ControlType, Duration as S2Duration, Id, PowerForecast,
    PowerForecastElement, PowerForecastValue, PowerMeasurement, PowerValue, ResourceManagerDetails,
    Role, RoleType, SessionRequest, SessionRequestType,
};
use s2energy::websockets_json::S2Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Start the simple mock PV Panel on the given S2 connection.
pub async fn start_mock(mut connection: S2Connection) -> eyre::Result<()> {
    let simulator = PvSimulator::new();

    // Send ResourceManagerDetails to indicate some of our properties.
    let rm_details = ResourceManagerDetails {
        available_control_types: vec![ControlType::NotControlable],
        currency: None,
        firmware_version: Some("1.0.0".into()),
        instruction_processing_delay: S2Duration(1),
        manufacturer: Some("ACME, Inc.".into()),
        message_id: Id::generate(),
        model: Some("Generic PV Installation Model X".into()),
        name: Some("The Amazing ACEM, Inc. PV Installation Model X".into()),
        provides_forecast: true,
        provides_power_measurement_types: vec![CommodityQuantity::ElectricPowerL1],
        resource_id: Id::generate(),
        roles: vec![Role {
            commodity: Commodity::Electricity,
            role: RoleType::EnergyProducer,
        }],
        serial_number: Some("111-222-333-444-555".into()),
    };
    let control_type = connection.initialize_as_rm(rm_details).await?;
    if control_type != ControlType::NoSelection && control_type != ControlType::NotControlable {
        return Err(eyre!("The CEM wants a control type not supported by the simple PV simulator: {control_type:?}"));
    }

    // Send a power measurement every 60 seconds, and a new forecast every hour.
    let mut measurement_timer = tokio::time::interval(Duration::from_secs(60));
    let mut forecast_timer = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        tokio::select! {
            msg = connection.receive_message() => {
                // Usually we would process received instructions here, but as this PV is not controllable there
                // are no relevant messages for us to process.
                tracing::info!("Received message {msg:?}. Ignoring it, as this PV panel is not controllable.");
            }

            _ = measurement_timer.tick() => {
                let measurement_timestamp = Utc::now();
                let power_measurement = PowerMeasurement {
                    measurement_timestamp,
                    message_id: Id::generate(),
                    values: vec![PowerValue {
                        commodity_quantity: CommodityQuantity::ElectricPowerL1,
                        value: -simulator.get_current_power(), // Production is negative in S2, so -current_power.
                    }]
                };
                tracing::info!("Sending power measurement: {power_measurement:?}");
                connection.send_message(power_measurement).await?;
            }

            _ = forecast_timer.tick() => {
                let forecast_elements = simulator.get_24h_forecast().iter().map(|&forecast_value| {
                    PowerForecastElement {
                        duration: S2Duration(1000 * 60 * 60),
                        // Production is negative in S2, so -forecast_value.
                        power_values: vec![PowerForecastValue::new(CommodityQuantity::ElectricPowerL1, -forecast_value, None, None, None, None, None, None)]
                    }
                }).collect();
                let forecast = PowerForecast { elements: forecast_elements, message_id: Id::generate(), start_time: Utc::now() };
                tracing::info!("Sending power forecast: {forecast:?}");
                connection.send_message(forecast).await?;
            }

            _ = tokio::signal::ctrl_c() => {
                tracing::warn!("Received Ctrl-C signal, stopping simulation.");
                break;
            }
        }
    }

    connection.send_message(SessionRequest {
        diagnostic_label: Some("Session terminated by user (Ctrl-C)".into()),
        message_id: Id::generate(),
        request: SessionRequestType::Terminate,
    }).await?;

    Ok(())
}

/// The profile is scaled from 0.0 to 1.0, so we use this multiplier to turn it into Watts.
const POWER_IN_W: f64 = 2000.;

/// A very simple simulator for a PV panel.
/// 
/// This can be used to retrieve current power generation and a 24h forecast.
/// In real usecases, this would be replaced by communication with the inverter or panel itself.
struct PvSimulator {
    profile: HashMap<DateTime<Utc>, f64>,
    /// The delta between real time and simulated time.
    time_delta: TimeDelta,
}

impl PvSimulator {
    pub fn new() -> Self {
        // Read the simulated values from a profile.
        let mut csv_reader = csv::Reader::from_reader(include_str!("solar.csv").as_bytes());
        let profile = csv_reader
            .deserialize()
            .filter_map(|result: Result<ProfileRow, _>| result.ok())
            .map(|row| (row.timestamp, row.value))
            .collect();

        // Calculate the time delta between simulated and real time.
        let simulated_start_time: DateTime<Utc> =
            DateTime::parse_from_rfc3339("2030-01-01T12:00:00Z")
                .unwrap()
                .into();
        let time_delta = simulated_start_time - Utc::now();

        Self {
            profile,
            time_delta,
        }
    }

    pub fn get_current_power(&self) -> f64 {
        let simulated_current_time = Utc::now() + self.time_delta;
        let rounded_time = simulated_current_time
            .duration_round(TimeDelta::hours(1))
            .unwrap();
        *self.profile.get(&rounded_time).unwrap() * POWER_IN_W
    }

    /// Returns a 24h forecast: a `Vec` with 24 elements, one for each hour in order, starting at the next hour.
    pub fn get_24h_forecast(&self) -> Vec<f64> {
        let simulated_current_time = Utc::now() + self.time_delta;
        let rounded_time = simulated_current_time
            .duration_round(TimeDelta::hours(1))
            .unwrap();

        (0..24)
            .map(|offset| {
                let offset_time = rounded_time + TimeDelta::hours(offset + 1);
                *self.profile.get(&offset_time).unwrap() * POWER_IN_W
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProfileRow {
    timestamp: DateTime<Utc>,
    value: f64,
}
