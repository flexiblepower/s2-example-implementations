# PV Installation

<div align="center">
    <img src="./solar-panel.svg" width="200" height="200" />
</div>
<br />

This example implementation simulates a PV installation of 2000 Wp. The curtailable (PEBC) implementation is contained in `src/pv_simulator_pebc.rc`, and the non-curtailable (NOT_CONTROLABLE) implementation is in `src/pv_simulator_simple.rs`. They both use the data from `src/solar.csv` to simulate solar production; to make sure you always have some interesting production data, they start at 2030-01-01 12:00:00 in the profile. That's useful when you're debugging late at night, when real solar production would be 0.

For more information on using the example implementations, look at the [README](../README.md) in the project root. We also have [an implementation guide for PV installations](https://docs.s2standard.org/docs/examples/pv/) in our documentation that may be useful to you.
