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
edge matches a provider actually delivering to that consumer.

*Spike S10: 100 edges drawn, 0 differing from the deliveries.*

## §4 Wildcard puts (R6)

**Steps.** A principal puts 10 samples on
`zk2/vehicle-01/*/detections.v1/stream/objects`.

**Expected.** A bound consumer receives them with the wildcard key, and
discards all 10.

*Spike S10: 10 of 10 discarded; spike S1: about 10 ns per check.*
