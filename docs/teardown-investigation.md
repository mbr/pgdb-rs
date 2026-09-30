# PostgreSQL teardown investigation

Elephant's [successful run 36739861821](https://github.com/mbr/elephant-rs/actions/runs/36739861821)
at `b24e1e2a89f8efd93fce762bbb9297fa81980ac6` used `pgdb 0.9.0`, `process_guard 0.4.0`,
and PostgreSQL 17.10. Logs retrieved with
`gh run view 36739861821 --repo mbr/elephant-rs --log` showed:

- At `15:54:27.049 UTC`, postmaster `54059` received fast shutdown; the runner timestamp was
  `15:54:27.0500720`.
- At `15:54:27.057 UTC`, checkpointer `54061` began its shutdown checkpoint.
- At `15:54:32.169 UTC`, that checkpointer logged
  `PANIC: could not fsync file "base/17298/2608": No such file or directory`.
- All 113 enabled integration tests passed. This panic was not intentional fault injection.

## Mechanism and evidence

Elephant called `pgdb::db_fixture()` without enabling fast mode. Overlapping fixtures shared a
local temporary cluster. Dropping its last owner requested `SIGINT`, which disconnects clients
but still performs a shutdown checkpoint. The guard allowed five seconds before sending
`SIGKILL` to the postmaster's process group.

PostgreSQL children call `setsid()` and occupy separate process groups. `process_guard` can
successfully kill and reap the postmaster and observe its group disappear while a worker remains
alive elsewhere. Its `Ok(Some(status))` reports completion of group shutdown, even when the exit
status is `SIGKILL`. `pgdb` then removes the temporary directory. A worker's later panic is not
returned to the Rust test harness.

A focused Linux reproducer stopped the checkpointer with `SIGSTOP`, recorded PIDs/groups/sessions,
and dropped the server using the original timeout. With the pinned historical dependencies and
PostgreSQL 17.10, drop returned after `5.118s` with the worker still stopped and its working directory
marked deleted. The same mechanism reproduced on the unmodified `0.10.0` checkout with PostgreSQL
17.11. The harness ran as a private subreaper and killed/reaped survivors before its own cleanup.

A traced historical run recorded postmaster PID/PGID `2326616` and checkpointer PID/PGID/SID
`2326618`. Selected syscall timestamps, with the wait status abbreviated:

```text
1790790627.733887 kill(2326616, SIGINT) = 0
1790790632.734077 kill(-2326616, SIGKILL) = 0
1790790632.834224 wait4(2326616, [SIGKILL], WNOHANG, NULL) = 2326616
1790790632.834297 kill(-2326616, 0) = -1 ESRCH
1790790632.872850 unlinkat(3, "db", AT_REMOVEDIR) = 0
```

This confirms the cleanup race, not the exact CI sequence. The original logs do not directly
record forced termination or explain why its checkpoint exceeded five seconds. The reproducer
held a worker stopped rather than reproducing the original slow checkpoint.

## Selected changes

Preserving temporary directories whenever the postmaster exited abnormally was implemented and
then reverted. That approach avoided deletion beneath a worker but left disposable directories
behind without fixing worker termination. The data itself does not need preservation.

Instead, local `db_fixture()` servers and disposable CLI servers now default to fast mode. It
disables durability writes and requests PostgreSQL's immediate shutdown with `SIGQUIT`, avoiding
a checkpoint for data that will be discarded. `PGDB_FAST=false` opts out; explicit CLI `--fast`
or `--test` takes precedence. Persistent CLI directories retain normal defaults, direct library
builders remain normal by default, and external/attached servers are not reconfigured.

The default grace period is five seconds for fast mode and twenty seconds for normal mode.
Explicit timeouts override either default, independent of builder call order. The existing
forceful-shutdown timeout and error handling are unchanged.

Regression coverage lives in `pgdb/tests/fixture.rs`, `pgdb_cli/tests/fast_mode.rs`, and the builder
and environment unit tests. It checks durability settings, shutdown requests, cleanup, persistent
and external server behavior, and override precedence. This addresses the disposable-server
defaults rather than promising process-tree containment.

## Remaining limitation

A stopped or unresponsive worker outside the postmaster's group can still survive forced
shutdown. Neither a longer timeout nor immediate shutdown provides an unconditional termination
guarantee when PostgreSQL cannot coordinate its workers. That needs separate containment or
worker-supervision work and a regression that independently cleans up survivors on failure.
The successful-run panic is therefore evidence of a real lifecycle limitation, not a failed CI run
or proof that these defaults solve every forced-shutdown case.
