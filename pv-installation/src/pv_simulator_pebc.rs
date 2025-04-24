use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use eyre::eyre;
use s2energy::common::{
    Commodity, CommodityQuantity, ControlType, Duration as S2Duration, Id, InstructionStatus,
    InstructionStatusUpdate, Message, NumberRange, PowerForecast, PowerForecastElement,
    PowerForecastValue, PowerMeasurement, PowerValue, ResourceManagerDetails, Role, RoleType,
    SessionRequest, SessionRequestType,
};
use s2energy::pebc;
use s2energy::websockets_json::S2Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Start the PEBC mock PV Panel on the given S2 connection.
pub async fn start_mock(mut connection: S2Connection) -> eyre::Result<()> {
    let mut simulator = PvSimulator::new();

    // Send ResourceManagerDetails to indicate some of our properties.
    let rm_details = ResourceManagerDetails {
        available_control_types: vec![ControlType::PowerEnvelopeBasedControl],
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
    if control_type != ControlType::PowerEnvelopeBasedControl {
        return Err(eyre!(
            "The CEM wants a control type not supported by the PEBC PV simulator: {control_type:?}"
        ));
    }

    // Communicate our power constraints to the CEM: in this example, we can always fully curtail our power.
    let power_constraints = pebc::PowerConstraints {
        allowed_limit_ranges: vec![
            pebc::AllowedLimitRange {
                // Upper limit
                abnormal_condition_only: false,
                commodity_quantity: CommodityQuantity::ElectricPowerL1,
                limit_type: pebc::PowerEnvelopeLimitType::UpperLimit,
                range_boundary: NumberRange::new(0.0, 0.0),
            },
            pebc::AllowedLimitRange {
                // Lower limit
                abnormal_condition_only: false,
                commodity_quantity: CommodityQuantity::ElectricPowerL1,
                limit_type: pebc::PowerEnvelopeLimitType::LowerLimit,
                range_boundary: NumberRange {
                    start_of_range: 0.0,
                    end_of_range: -POWER_IN_W,
                },
            },
        ],
        consequence_type: pebc::PowerEnvelopeConsequenceType::Vanish,
        id: Id::generate(),
        message_id: Id::generate(),
        valid_from: Utc::now(),
        valid_until: None,
    };
    connection.send_message(power_constraints).await?;

    // Send a power measurement every 60 seconds, and a new forecast every hour.
    let mut measurement_timer = tokio::time::interval(Duration::from_secs(60));
    let mut forecast_timer = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        tokio::select! {
            msg = connection.receive_message() => {
                let instruction = match msg? {
                    Message::PebcInstruction(instruction) => instruction,
                    msg => {
                        tracing::info!("Received message {msg:?}. Ignoring it, as it's not a PEBC.Instruction.");
                        continue;
                    }
                };

                // Store any power envelopes received.
                let base_time = instruction.execution_time;
                for envelope in &instruction.power_envelopes {
                    if envelope.commodity_quantity != CommodityQuantity::ElectricPowerL1 {
                        tracing::warn!("Received power envelope for irrelevant commodity quantity {:?}", envelope.commodity_quantity);
                        continue;
                    }

                    for element in &envelope.power_envelope_elements {
                        let end_time = base_time + TimeDelta::milliseconds(element.duration.0 as i64);
                        simulator.add_constraint(base_time, end_time, element.lower_limit, element.upper_limit);
                    }
                }

                // Confirm receipt and acceptance of the instruction.
                let instruction_status = InstructionStatusUpdate {
                    instruction_id: instruction.id,
                    message_id: Id::generate(),
                    status_type: InstructionStatus::Succeeded,
                    timestamp: Utc::now()
                };
                connection.send_message(instruction_status).await?;
            }

            _ = measurement_timer.tick() => {
                // Send a measurement of current power production.
                let measurement_timestamp = Utc::now();
                let power_measurement = PowerMeasurement {
                    measurement_timestamp,
                    message_id: Id::generate(),
                    values: vec![PowerValue {
                        commodity_quantity: CommodityQuantity::ElectricPowerL1,
                        value: simulator.get_current_power(),
                    }]
                };
                tracing::info!("Sending power measurement: {power_measurement:?}");
                connection.send_message(power_measurement).await?;
            }

            _ = forecast_timer.tick() => {
                // Send a new forecast for the next 24 hours.
                let forecast_elements = simulator.get_24h_forecast().iter().map(|&forecast_value| {
                    PowerForecastElement {
                        duration: S2Duration(1000 * 60 * 60),
                        power_values: vec![PowerForecastValue::new(CommodityQuantity::ElectricPowerL1, forecast_value, None, None, None, None, None, None)]
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

    connection
        .send_message(SessionRequest {
            diagnostic_label: Some("Session terminated by user (Ctrl-C)".into()),
            message_id: Id::generate(),
            request: SessionRequestType::Terminate,
        })
        .await?;

    Ok(())
}

/// The profile is scaled from 0.0 to 1.0, so we use this multiplier to turn it into Watts.
const POWER_IN_W: f64 = 2000.;

struct PvConstraint {
    lower_limit: f64,
    upper_limit: f64,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
}

/// A very simple simulator for a PV panel.
///
/// This can be used to retrieve current power generation and a 24h forecast.
/// In real usecases, this would be replaced by communication with the inverter or panel itself.
struct PvSimulator {
    profile: HashMap<DateTime<Utc>, f64>,
    /// The delta between real time and simulated time.
    time_delta: TimeDelta,
    /// Any constraints on our power output (as derived from instructions received by the RM).
    constraints: Vec<PvConstraint>,
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
            constraints: Vec::new(),
        }
    }

    pub fn get_current_power(&self) -> f64 {
        let simulated_current_time = Utc::now() + self.time_delta;
        let rounded_time = simulated_current_time
            .duration_round(TimeDelta::hours(1))
            .unwrap();

        let (lower_limit, upper_limit) = self.get_current_constraints();

        self.profile
            .get(&rounded_time)
            .unwrap()
            .max(lower_limit)
            .min(upper_limit)
            * POWER_IN_W
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
                self.profile
                    .get(&offset_time)
                    .unwrap()
                    * POWER_IN_W
            })
            .collect()
    }

    fn get_current_constraints(&self) -> (f64, f64) {
        for constraint in &self.constraints {
            if constraint.start_time <= Utc::now() && constraint.end_time >= Utc::now() {
                return (constraint.lower_limit, constraint.upper_limit);
            }
        }

        (-1.0, 1.0)
    }

    pub fn add_constraint(
        &mut self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        lower_limit: f64,
        upper_limit: f64,
    ) {
        self.constraints.push(PvConstraint {
            lower_limit: lower_limit / POWER_IN_W,
            upper_limit: upper_limit / POWER_IN_W,
            start_time,
            end_time,
        });
        // Also clean up any old constraints that have already ended.
        self.constraints
            .retain(|constraint| constraint.end_time > Utc::now());
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProfileRow {
    timestamp: DateTime<Utc>,
    value: f64,
}
