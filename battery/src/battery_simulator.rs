use chrono::{DateTime, Utc};
use eyre::{Context, Result, eyre};
use maplit::hashmap;
use s2energy::common::{
    Commodity, CommodityQuantity, ControlType, Duration as S2Duration, Id, InstructionStatus,
    InstructionStatusUpdate, Message, NumberRange, PowerRange, ResourceManagerDetails, Role,
    Transition,
};
use s2energy::frbc::{self, LeakageBehaviourElement, OperationMode, OperationModeElement};
use s2energy::websockets_json::S2Connection;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Duration;

pub async fn start_mock(mut connection: S2Connection) -> eyre::Result<()> {
    let mut simulator = Simulator::new();

    connection
        .initialize_as_rm(ResourceManagerDetails {
            available_control_types: vec![ControlType::FillRateBasedControl],
            currency: None,
            firmware_version: None,
            instruction_processing_delay: s2energy::common::Duration(10),
            manufacturer: None,
            message_id: Id::generate(),
            model: None,
            name: None,
            provides_forecast: true,
            provides_power_measurement_types: vec![CommodityQuantity::ElectricPower3PhaseSymmetric],
            resource_id: Id::generate(),
            roles: vec![Role::new(
                s2energy::common::Commodity::Electricity,
                s2energy::common::RoleType::EnergyConsumer,
            )],
            serial_number: None,
        })
        .await
        .wrap_err("Error communicating initial info with CEM")?;

    // Send the initial info that the CEM needs: a system description, a leakage behaviour, and a forecast
    connection
        .send_message(simulator.system_description())
        .await?;
    connection
        .send_message(simulator.leakage_behaviour())
        .await?;
    connection.send_message(simulator.forecast()).await?;

    let mut update_timer = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            message = connection.receive_message() => {
                let message = message?;
                let updates = simulator.process_message(&message)?;
                for update in updates {
                    connection.send_message(update).await?;
                }
            },

            _ = update_timer.tick() => {
                // Send a StorageStatus message every 60 seconds
                let update = simulator.update();
                connection.send_message(update).await?;
            }

            _ = tokio::signal::ctrl_c() => {
                tracing::warn!("Received Ctrl-C signal, stopping simulation.");
                break;
            }
        }
    }

    Ok(())
}

const CHARGE_EFFICIENCY: f64 = 1.0;
const DISCHARGE_EFFICIENCY: f64 = 1.0;
const CAPACITY_WH: f64 = 20_000.0;
const LEAKAGE_W: f64 = 0.5;
const INITIAL_FILL_LEVEL: f64 = 0.5;

// Generate the IDs for our operation modes.
// These should be kept consistent during the simulation, so that's why they're const here.
const OPERATION_MODE_IDLE: LazyLock<Id> =
    LazyLock::new(|| Id::from_str(&uuid::Uuid::new_v4().to_string()).unwrap());
const OPERATION_MODE_CHARGE: LazyLock<Id> =
    LazyLock::new(|| Id::from_str(&uuid::Uuid::new_v4().to_string()).unwrap());
const OPERATION_MODE_DISCHARGE: LazyLock<Id> =
    LazyLock::new(|| Id::from_str(&uuid::Uuid::new_v4().to_string()).unwrap());
const ACTUATOR_1: LazyLock<Id> =
    LazyLock::new(|| Id::from_str(&uuid::Uuid::new_v4().to_string()).unwrap());

pub struct Simulator {
    pub operation_modes: HashMap<Id, OperationMode>,
    fill_level: f64,
    active_operation_mode: Id,
    operation_mode_factor: f64,
    simulation_start: DateTime<Utc>,
    last_updated: DateTime<Utc>,
}

impl Simulator {
    pub fn new() -> Self {
        // Define the three operation modes: idle, charging, discharging.
        let operation_mode_idle = OperationMode {
            abnormal_condition_only: false,
            diagnostic_label: Some("Idle".into()),
            elements: vec![OperationModeElement {
                running_costs: None,
                fill_rate: NumberRange {
                    start_of_range: 0.0,
                    end_of_range: 0.0,
                },
                fill_level_range: NumberRange {
                    start_of_range: 0.0,
                    end_of_range: 1.0,
                },
                power_ranges: vec![PowerRange {
                    commodity_quantity: CommodityQuantity::ElectricPower3PhaseSymmetric,
                    start_of_range: 0.,
                    end_of_range: 0.,
                }],
            }],
            id: OPERATION_MODE_IDLE.clone(),
        };

        let operation_mode_charge = OperationMode {
            abnormal_condition_only: false,
            diagnostic_label: Some("Charging battery".into()),
            elements: vec![OperationModeElement {
                running_costs: None,
                fill_rate: NumberRange {
                    start_of_range: CHARGE_EFFICIENCY * ((5000.0 / CAPACITY_WH) / 3600.),
                    end_of_range: 0.5 * CHARGE_EFFICIENCY * (5000.0 / CAPACITY_WH / 3600.),
                },
                fill_level_range: NumberRange {
                    start_of_range: 0.0,
                    end_of_range: 1.0,
                },
                power_ranges: vec![PowerRange {
                    commodity_quantity: CommodityQuantity::ElectricPower3PhaseSymmetric,
                    start_of_range: 5000.,
                    end_of_range: 0.5 * 5000.,
                }],
            }],
            id: OPERATION_MODE_CHARGE.clone(),
        };

        let operation_mode_discharge = OperationMode {
            abnormal_condition_only: false,
            diagnostic_label: Some("Discharging battery".into()),
            elements: vec![OperationModeElement {
                running_costs: None,
                fill_rate: NumberRange {
                    start_of_range: DISCHARGE_EFFICIENCY * ((5000.0 / CAPACITY_WH) / 3600.),
                    end_of_range: 0.5 * DISCHARGE_EFFICIENCY * (5000.0 / CAPACITY_WH / 3600.),
                },
                fill_level_range: NumberRange {
                    start_of_range: 0.0,
                    end_of_range: 1.0,
                },
                power_ranges: vec![PowerRange {
                    commodity_quantity: CommodityQuantity::ElectricPower3PhaseSymmetric,
                    start_of_range: -5000.,
                    end_of_range: 0.5 * -5000.,
                }],
            }],
            id: OPERATION_MODE_DISCHARGE.clone(),
        };

        Self {
            fill_level: INITIAL_FILL_LEVEL,
            operation_modes: hashmap! {
                OPERATION_MODE_IDLE.clone() => operation_mode_idle,
                OPERATION_MODE_CHARGE.clone() => operation_mode_charge,
                OPERATION_MODE_DISCHARGE.clone() => operation_mode_discharge,
            },
            active_operation_mode: OPERATION_MODE_IDLE.clone(),
            operation_mode_factor: 0.5,
            simulation_start: Utc::now(),
            last_updated: Utc::now(),
        }
    }

    pub fn system_description(&self) -> frbc::SystemDescription {
        // Define our storage properties.
        let storage_description = frbc::StorageDescription {
            diagnostic_label: Some("Battery".into()),
            fill_level_label: Some("Fraction, 0.0 to 1.0".into()),
            fill_level_range: NumberRange {
                start_of_range: 0.0,
                end_of_range: 1.0,
            },
            provides_fill_level_target_profile: false,
            provides_leakage_behaviour: true,
            provides_usage_forecast: true,
        };

        let actuator_description = frbc::ActuatorDescription {
            diagnostic_label: None,
            id: ACTUATOR_1.clone(),
            operation_modes: self
                .operation_modes
                .iter()
                .map(|(_, mode)| mode.clone())
                .collect(),
            supported_commodities: vec![Commodity::Electricity],
            timers: vec![],
            transitions: vec![
                // Idle <--> charging
                Transition::new(
                    false,
                    vec![],
                    OPERATION_MODE_IDLE.clone(),
                    Id::generate(),
                    vec![],
                    OPERATION_MODE_CHARGE.clone(),
                    None,
                    None,
                ),
                Transition::new(
                    false,
                    vec![],
                    OPERATION_MODE_CHARGE.clone(),
                    Id::generate(),
                    vec![],
                    OPERATION_MODE_IDLE.clone(),
                    None,
                    None,
                ),
                // Idle <--> discharging
                Transition::new(
                    false,
                    vec![],
                    OPERATION_MODE_IDLE.clone(),
                    Id::generate(),
                    vec![],
                    OPERATION_MODE_DISCHARGE.clone(),
                    None,
                    None,
                ),
                Transition::new(
                    false,
                    vec![],
                    OPERATION_MODE_DISCHARGE.clone(),
                    Id::generate(),
                    vec![],
                    OPERATION_MODE_IDLE.clone(),
                    None,
                    None,
                ),
            ],
        };

        frbc::SystemDescription::new(vec![actuator_description], storage_description, Utc::now())
    }

    pub fn update(&mut self) -> frbc::StorageStatus {
        // Update the fill level based on our current operation mode
        let delta_time = Utc::now() - self.last_updated;
        self.last_updated = Utc::now();

        let fill_rates = &self.operation_modes[&self.active_operation_mode].elements[0].fill_rate;
        let fill_rate = fill_rates.start_of_range
            + (fill_rates.end_of_range - fill_rates.start_of_range) * self.operation_mode_factor;
        self.fill_level += fill_rate * delta_time.num_seconds() as f64;
        self.fill_level = self.fill_level.clamp(0.0, 1.0);

        frbc::StorageStatus::new(self.fill_level)
    }

    pub fn leakage_behaviour(&self) -> frbc::LeakageBehaviour {
        frbc::LeakageBehaviour {
            elements: vec![LeakageBehaviourElement {
                fill_level_range: NumberRange {
                    start_of_range: 0.0,
                    end_of_range: 1.0,
                },
                leakage_rate: (LEAKAGE_W / CAPACITY_WH) / 3600.,
            }],
            message_id: Id::generate(),
            valid_from: Utc::now(),
        }
    }

    pub fn forecast(&self) -> frbc::UsageForecast {
        // This is a home battery (i.e. not an EV battery), so we don't expect any usage
        frbc::UsageForecast::new(
            vec![
                frbc::UsageForecastElement {
                    duration: S2Duration(1000 * 3600),
                    usage_rate_expected: 0.,
                    usage_rate_lower_68ppr: None,
                    usage_rate_lower_95ppr: None,
                    usage_rate_lower_limit: None,
                    usage_rate_upper_68ppr: None,
                    usage_rate_upper_95ppr: None,
                    usage_rate_upper_limit: None,
                };
                24
            ],
            Utc::now(),
        )
    }

    pub fn process_message(&mut self, msg: &Message) -> Result<Vec<Message>> {
        // Ensure our fill level is always up-to-date
        let storage_status = self.update();

        let last_operation_mode = self.active_operation_mode.clone();
        if let Message::FrbcInstruction(instruction) = msg {
            if self
                .operation_modes
                .contains_key(&instruction.operation_mode)
            {
                // Switch operation modes and adjust the operation mode factor
                self.active_operation_mode = instruction.operation_mode.clone();
                self.operation_mode_factor = instruction.operation_mode_factor;
            } else {
                // CEM requested a nonexistent operation mode, so report back an error
                let status = InstructionStatusUpdate {
                    instruction_id: msg.id().unwrap(),
                    message_id: Id::generate(),
                    status_type: InstructionStatus::Rejected,
                    timestamp: Utc::now(),
                };
                return Ok(vec![status.into()]);
            }
        } else {
            // Ignore any messagess we get that aren't FRBC.Instruction
            return Ok(vec![]);
        }

        // Send the CEM back our current status after switching operation modes
        let instruction_status = InstructionStatusUpdate {
            instruction_id: msg.id().unwrap(),
            message_id: Id::generate(),
            status_type: InstructionStatus::Succeeded,
            timestamp: Utc::now(),
        };

        let actuator_status = frbc::ActuatorStatus {
            active_operation_mode_id: self.active_operation_mode.clone(),
            actuator_id: ACTUATOR_1.clone(),
            message_id: Id::generate(),
            operation_mode_factor: self.operation_mode_factor,
            previous_operation_mode_id: Some(last_operation_mode),
            transition_timestamp: Some(Utc::now()),
        };

        Ok(vec![
            instruction_status.into(),
            actuator_status.into(),
            storage_status.into(),
        ])
    }
}
