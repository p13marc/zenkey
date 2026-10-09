# Binding scenarios (core §3.2)

## §1 Static bindings and fan-in (R1, R5)

**Setup.** A tracker binds `detections.v1` as `sources` to `vehicle-01/*`.
100 detectors start and stop.

**Steps.**
1. Start the tracker before any detector.
2. Start the detectors one by one, then stop half of them.

**Expected.**
1. The binding resolves at once, without presence.
2. Samples from every running detector are delivered. A consumer waiting on
   presence starts within one token propagation of a provider's arrival.

*Spike S10: fan-in to 100 with 0 gaps; join within 11 ms.*

## §2 Binding a parameter to self (R2)

**Setup.** `vehicle-01/executor` binds `mission_plan.v1` as `plan` to
`ground/fleet-mgr`, with `{vehicle} = self.system`.

**Expected.** The executor reads and subscribes only to
`…/plans/vehicle-01`.

## §3 The graph from descriptors (R3)

**Steps.** A tool reads every descriptor and interface token, and nothing
else.

**Expected.** The tool draws every edge (consumer role → provider). Each
edge matches a provider actually delivering to that consumer. A role that
an owner's configuration leaves unbound, optional ones included, is in its
descriptor with `"bindings": []`, so the tool also sees the edges a contract
declares and nothing serves (core §3.2).

*Spike S10: 100 edges drawn, 0 differing from the deliveries.*

## §4 Wildcard puts (R6)

**Steps.** A principal puts 10 samples on
`zk2/vehicle-01/*/detections.v1/stream/objects`.

**Expected.** A bound consumer receives them with the wildcard key, and
discards all 10.

*Spike S10: 10 of 10 discarded; spike S1: about 10 ns per check.*

## §5 A provider on the service's own system (R1, R3; 0.20)

**Setup.** Detectors `vehicle-01/det0`, `vehicle-01/det1` and
`vehicle-02/det0` implement `detections.v1`. Two trackers on
`vehicle-01` bind `detections.v1` as `sources`: `vehicle-01/one` to
`self.system/det0`, and `vehicle-01/all` to `self.system/*`.

**Steps.**
1. Every detector puts samples on `stream/objects`.
2. A tool reads both trackers' descriptors.
3. A tool, which is not a service, binds a consumer of its own to
   `self.system/det0`.

**Expected.**
1. `vehicle-01/one` receives from `vehicle-01/det0` alone, and
   `vehicle-01/all` from `vehicle-01/det0` and `vehicle-01/det1`. Neither
   receives from `vehicle-02/det0`.
2. The descriptors list the bindings resolved: `["vehicle-01/det0"]` and
   `["vehicle-01/*"]`.
3. The tool's binding is refused: a tool has no system of its own.

A system that a profile derives resolves the same way, from the derived
system: `hostid.v1`'s scenarios bind `self.system/sysinfo` from a service
whose system is minted (`profiles/hostid/scenarios.md` §5).
