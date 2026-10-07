# S11 — control arbitration (#594)

Written by `spike s11`; the latest run (`unix_s` 1791410628), zenoh 1.10.1. Deadline 100 ms and lifespan 100 ms from `twist_cmd.v1`'s `timing.v1` annotations.

| Mode | Phase | Value | Pass |
|---|---|---|---|
| P3 | autopilot alone: both actuators follow it | true | yes |
| P3 | teleop takes over: first teleop send to switch, ms (l, r) | 0.26, 0.27 | yes |
| P3 | teleop takes over: the two actuators switch within, ms | 0.02 | yes |
| P3 | teleop silent: last teleop command to autopilot resuming, ms (l, r) | 101.2, 100.1 | yes |
| P3 | teleop silent: detected by | deadline, deadline | yes |
| P3 | teleop resumes: both actuators back on teleop | true | yes |
| P3 | teleop crash (kill -9): signal to autopilot, ms (l, r) | 4.9, 5.0 | yes |
| P3 | teleop crash: detected by | liveliness, liveliness | yes |
| P3 | all silent: last command to dead-man stop, ms (l, r) | 101.5, 100.4 | yes |
| P3 | safety preempts autopilot: spawn to switch, ms | 7.5 | yes |
| P3 | steady, 3 commanders x 50 Hz x 2 actuators: 900 samples in 3 s; CPU of the process holding both actuators (2 ms evaluation tick each), % of one core | 7.0 | yes |
| P3 | false stops over the whole scenario | 0 | yes |
| P3 | skew -250 ms on autopilot: a 100 ms sender-clock lifespan check rejects (of the skewed samples) | 100/100 | yes |
| P3 | skew -250 ms on autopilot: the receive-clock deadline still follows it | true | yes |
| P3 | skew +250 ms on teleop: a 100 ms sender-clock lifespan check rejects (of the skewed samples) | 100/100 | yes |
| P3 | skew +250 ms on teleop: the receive-clock deadline still follows it | true | yes |
| P2 | autopilot alone: both actuators follow it | true | yes |
| P2 | teleop takes over: first teleop send to switch, ms (l, r) | 0.33, 0.34 | yes |
| P2 | teleop takes over: the two actuators switch within, ms | 0.01 | yes |
| P2 | teleop silent: last teleop command to autopilot resuming, ms (l, r) | 102.2, 101.1 | yes |
| P2 | teleop silent: detected by | deadline, deadline | yes |
| P2 | teleop resumes: both actuators back on teleop | true | yes |
| P2 | teleop crash (kill -9): signal to autopilot, ms (l, r) | 3.8, 3.8 | yes |
| P2 | teleop crash: detected by | liveliness, liveliness | yes |
| P2 | all silent: last command to dead-man stop, ms (l, r) | 101.5, 100.4 | yes |
| P2 | safety preempts autopilot: spawn to switch, ms | 9.2 | yes |
| P2 | steady, 3 commanders x 50 Hz x 2 actuators: 898 samples in 3 s; CPU of the process holding both actuators (2 ms evaluation tick each), % of one core | 7.7 | yes |
| P2 | false stops over the whole scenario | 0 | yes |
