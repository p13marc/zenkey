# S12 — store-and-forward (#595)

Written by `spike s12`; the latest run (`unix_s` 1791412804), zenoh 1.10.1 with the patched storage manager 1.10.1. **4 wrong answers** in 22 rows.

| Storage | Phase | Truth | Applied | Wrong | Note |
|---|---|---|---|---|---|
| storage on the ground | online: rev1 | rev1 | rev1 |  | converged in 0 ms |
| storage on the ground | offline, plan changed once, then the link heals | rev2 | rev2 |  | converged in 1589 ms; applied while offline: rev1; ground sees observed = rev2 1.9 s after the heal |
| storage on the ground | offline, plan changed 10 times, then the link heals | rev12 | rev12 |  | converged in 1396 ms; applied while offline: rev2; ground sees observed = rev12 1.7 s after the heal |
| storage on the ground | offline, plan deleted, then the link heals | (deleted) | (deleted) |  | converged in 1587 ms; applied while offline: rev12; ground sees observed = none 1.9 s after the heal |
| storage on the ground | online again: rev13 | rev13 | rev13 |  | converged in 10 ms |
| storage on the ground | executor restart | rev13 | rev13 |  | converged in 0 ms |
| storage on the ground | fleet manager restarts 5 s behind, naive | rev14 | rev13 | **WRONG** | did not converge in 15 s; stale-command rejections so far: 1 |
| storage on the ground | fleet manager restarts 5 s behind, with U2's catch-up | rev15 | rev15 |  | converged in 11 ms; stale-command rejections so far: 1 |
| storage on the ground | a fleet manager 2 s ahead writes rev16 (a GET reads it back at +2 s); 0.5 s later the right-clocked one writes rev17 | rev17 | rev16 | **WRONG** | did not converge in 15 s; stale-command rejections so far: 5 |
| storage on the ground | the next write once the 2 s skew has passed (rev18) | rev18 | rev18 |  | converged in 11 ms |
| storage on the ground | stale revisions applied over the whole run | 0 | 0 |  |  |
| storage on the vehicle | online: rev1 | rev1 | rev1 |  | converged in 0 ms |
| storage on the vehicle | offline, plan changed once, then the link heals | rev2 | rev2 |  | converged in 1588 ms; applied while offline: rev1; ground sees observed = rev2 1.9 s after the heal |
| storage on the vehicle | offline, plan changed 10 times, then the link heals | rev12 | rev12 |  | converged in 1397 ms; applied while offline: rev2; ground sees observed = rev12 1.7 s after the heal |
| storage on the vehicle | offline, plan deleted, then the link heals | (deleted) | (deleted) |  | converged in 1585 ms; applied while offline: rev12; ground sees observed = none 1.9 s after the heal |
| storage on the vehicle | online again: rev13 | rev13 | rev13 |  | converged in 11 ms |
| storage on the vehicle | executor restart | rev13 | rev13 |  | converged in 0 ms |
| storage on the vehicle | fleet manager restarts 5 s behind, naive | rev14 | rev13 | **WRONG** | did not converge in 15 s; stale-command rejections so far: 1 |
| storage on the vehicle | fleet manager restarts 5 s behind, with U2's catch-up | rev15 | rev15 |  | converged in 11 ms; stale-command rejections so far: 1 |
| storage on the vehicle | a fleet manager 2 s ahead writes rev16 (a GET reads it back at +2 s); 0.5 s later the right-clocked one writes rev17 | rev17 | rev16 | **WRONG** | did not converge in 15 s; stale-command rejections so far: 5 |
| storage on the vehicle | the next write once the 2 s skew has passed (rev18) | rev18 | rev18 |  | converged in 11 ms |
| storage on the vehicle | stale revisions applied over the whole run | 0 | 0 |  |  |
