# S14 — the ownership ACL (#616)

Written by `spike s14`; the latest run (`unix_s` 1791415622), zenoh 1.10.1. usrpwd subjects; the generated router configs are beside this file.

| Posture | Check | Expected | Observed | Match |
|---|---|---|---|---|
| deny + allows | teleop puts on its own cmd: the thruster receives it | yes | yes | yes |
| deny + allows | teleop puts on autopilot's cmd key: the thruster receives it | no | no | yes |
| deny + allows | teleop puts on zk2/vehicle-01/*/twist_cmd.v1/stream/cmd: it reaches autopilot's subscription (wildcard key; R6 drops it) | no | no | yes |
| deny + allows | teleop subscribes to the thruster's status (not granted): it receives it | no | no | yes |
| deny + allows | the thruster sees the commanders' presence (4 distinct tokens) | 4 | 4 | yes |
| deny + allows | the frontend sees the vehicle's presence (not granted) | 0 | 0 | yes |
| deny + allows | the frontend GETs zk2/*/tc/tc.netif.v1/state/** (2 backends) | 2 | 2 | yes |
| deny + allows | the frontend calls h1's concrete config/eth0/set | 1 | 1 | yes |
| deny + allows | the frontend's wildcard call (fan-out forbidden): values / server refusals | 0 / 2 | 0 / 2 | yes |
| deny + allows | backend h1 puts on h2's state: the frontend's subscription receives it | no | no | yes |
| deny + allows | the frontend fetches a contract bundle from h1 | 1 | 1 | yes |
| deny + allows | vehicle-01's executor GETs its plan / vehicle-02's plan | 1 / 0 | 1 / 0 | yes |
| deny + allows | a client with no credentials connects | no | no | yes |
| allow + the same allows (U21) | teleop puts on its own cmd: the thruster receives it | yes | yes | yes |
| allow + the same allows (U21) | teleop puts on autopilot's cmd key: the thruster receives it | yes (allows are not evaluated) | yes | yes |
| allow + the same allows (U21) | teleop puts on zk2/vehicle-01/*/twist_cmd.v1/stream/cmd: it reaches autopilot's subscription (wildcard key; R6 drops it) | yes (allows are not evaluated) | yes | yes |
| allow + the same allows (U21) | teleop subscribes to the thruster's status (not granted): it receives it | yes (allows are not evaluated) | yes | yes |
| allow + the same allows (U21) | the thruster sees the commanders' presence (4 distinct tokens) | 4 | 4 | yes |
| allow + the same allows (U21) | the frontend sees the vehicle's presence (not granted) | 4 (allows are not evaluated) | 4 | yes |
| allow + the same allows (U21) | the frontend GETs zk2/*/tc/tc.netif.v1/state/** (2 backends) | 2 | 2 | yes |
| allow + the same allows (U21) | the frontend calls h1's concrete config/eth0/set | 1 | 1 | yes |
| allow + the same allows (U21) | the frontend's wildcard call (fan-out forbidden): values / server refusals | 0 / 2 | 0 / 2 | yes |
| allow + the same allows (U21) | backend h1 puts on h2's state: the frontend's subscription receives it | yes (allows are not evaluated) | yes | yes |
| allow + the same allows (U21) | the frontend fetches a contract bundle from h1 | 1 | 1 | yes |
| allow + the same allows (U21) | vehicle-01's executor GETs its plan / vehicle-02's plan | 1 / 1 (allows are not evaluated) | 1 / 1 | yes |
| allow + the same allows (U21) | a client with no credentials connects | no | no | yes |
| allow + denies of the complement (D13) | teleop puts on its own cmd: the thruster receives it | yes | yes | yes |
| allow + denies of the complement (D13) | teleop puts on autopilot's cmd key: the thruster receives it | no | no | yes |
| allow + denies of the complement (D13) | teleop puts on zk2/vehicle-01/*/twist_cmd.v1/stream/cmd: it reaches autopilot's subscription (wildcard key; R6 drops it) | yes (a wildcard is not included in any deny; R6 drops it) | yes | yes |
| allow + denies of the complement (D13) | teleop subscribes to the thruster's status (not granted): it receives it | no | no | yes |
| allow + denies of the complement (D13) | the thruster sees the commanders' presence (4 distinct tokens) | 4 | 4 | yes |
| allow + denies of the complement (D13) | the frontend sees the vehicle's presence (not granted) | 0 | 0 | yes |
| allow + denies of the complement (D13) | the frontend GETs zk2/*/tc/tc.netif.v1/state/** (2 backends) | 2 | 2 | yes |
| allow + denies of the complement (D13) | the frontend calls h1's concrete config/eth0/set | 1 | 1 | yes |
| allow + denies of the complement (D13) | the frontend's wildcard call (fan-out forbidden): values / server refusals | 0 / 2 | 0 / 2 | yes |
| allow + denies of the complement (D13) | backend h1 puts on h2's state: the frontend's subscription receives it | no | no | yes |
| allow + denies of the complement (D13) | the frontend fetches a contract bundle from h1 | 1 | 1 | yes |
| allow + denies of the complement (D13) | vehicle-01's executor GETs its plan / vehicle-02's plan | 1 / 0 | 1 / 0 | yes |
| allow + denies of the complement (D13) | a client with no credentials connects | no | no | yes |
