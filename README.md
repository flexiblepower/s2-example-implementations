# Example implementations of S2 devices

<div align="center">
    <a href="https://s2standard.org"><img src="./Logo-S2.svg" width="200" height="200" /></a>
</div>
<br />

This repository provides example implementations of S2 devices that you can use as example code or to test your own implementation. The provided example implementations are fully functioning implementations of S2 Resource Managers (RMs). Instead of a real, physical device, the RMs run a simulated device that provides data and responds to instructions.

## Testing against an implementation
These implementations are useful when testing your own S2 implementation: if you're developing a Customer Energy Manager (CEM), you can spin up one of the RMs in this repository to test that your CEM can succesfully connect and communicate with the RM. To do so, we recommend you use the provided `docker-compose.yml`; simply comment/uncomment the devices you want to test with and use the provided environment variables to configure the RMs.

Currently, we provide the following example implementations:
- `pv-installation` simulates a PV installation of 2000 Wp. It can simulate both a curtailable PV installation (`PEBC`) and a non-curtailable PV installation (`NOT_CONTROLABLE`).
- `battery` simulates a home battery with a capacity of 20 kWh. As it's a storage device, it implements `FRBC` and is a great way to test your `FRBC` implementation.