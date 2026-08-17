# Instance Recovery and Crash Loop Protection

Llama Server Manager can supervise an instance after a startup failure or an unexpected `llama-server` exit. Recovery is configured per instance and is disabled by default so an upgrade does not change existing lifecycle behavior.

## Enable recovery

Open **Instances**, select an instance, and enable **Failure Recovery** in the details panel. This saves `restart_policy: "on-failure"` in the instance configuration. Disable the switch to use `restart_policy: "never"`.

**Auto Start** and **Failure Recovery** are separate policies:

- Auto Start decides whether a stopped instance should start when the application starts.
- Failure Recovery decides whether the runtime service should restart an instance after a failed start or unexpected exit.
- Auto Start never overrides an active recovery incident or Crash Loop.

## Recovery contract

An operator stop, application shutdown, runtime upgrade handoff, or other expected lifecycle transition does not spend the recovery budget and does not trigger an automatic restart.

For `on-failure`, one incident has at most three automatic restart attempts:

| Automatic attempt | Backoff before start |
| --- | --- |
| 1 | 2 seconds |
| 2 | 10 seconds |
| 3 | 30 seconds |

If the third automatic attempt fails to start or exits before five minutes of stable runtime, the instance enters **Crash Loop** and no further automatic start is scheduled. A later failure after at least five minutes of continuous runtime begins a new incident with a fresh budget.

The fixed limits are deliberately not user-extensible in this phase. They provide one cross-platform safety contract and prevent an invalid executable, occupied port, broken model, or incompatible engine from creating an unbounded process loop.

## States and operator actions

| State | Meaning | Available operator action |
| --- | --- | --- |
| Recovering | A retry is waiting for backoff or a recovered process is inside the stability window. | **Cancel Recovery** stops the incident and removes its desired runtime intent. |
| Error | Recovery is disabled and the last attempted start or process run failed. | **Start** retries immediately. |
| Crash Loop | The bounded automatic retry budget is exhausted. | **Retry Now** resets the retry budget and starts immediately. |
| Running | The process is live. A recovery incident may remain visible until stability is proven. | **Stop** is always an expected stop and cancels recovery. |

Manual retry resets the automatic-attempt counter but retains the incident's originating failure. A deliberate Stop clears the active incident.

Saving a launch-affecting configuration change never rewrites the command snapshot of an already running process. While that snapshot is stale, an unexpected exit is recorded but is not automatically restarted with the old engine, model, port, or arguments. A waiting retry is cancelled immediately. Start the instance manually after saving to create a fresh launch snapshot and re-enable recovery. Manager-only edits such as the display name, Auto Start, or Failure Recovery policy do not make the snapshot stale.

## Diagnostics and persistence

The instance details panel records both:

- the immutable **originating failure** for the current incident; and
- the **latest failure** from the most recent retry.

Each record includes failure class (`startup_failure` or `unexpected_exit`), message, exit code when available, and time. Automatic retries never replace the originating failure. The normal capped instance log remains available for full `llama-server` output.

Desired runtime intent, retry count, next retry time, Crash Loop state, and failure evidence are written to the runtime-service state file using the existing atomic primary/backup persistence path. Restarting the GUI or runtime service therefore cannot silently reset the budget or bypass Crash Loop. Runtime-state schema 1 migrates to schema 2 with an empty recovery map; older binaries reject schema 2 instead of silently ignoring recovery state. Existing instance configurations default to `never`.

## Troubleshooting

1. Read the originating failure before retrying. Repeated retries do not repair a missing engine, invalid model, occupied port, or rejected security configuration.
2. Open the instance log for the complete process output.
3. Correct the engine, model, port, permissions, or launch parameters.
4. Choose **Retry Now** from Crash Loop, or **Start** from Error.
5. If no retry should occur, choose **Cancel Recovery** or disable Failure Recovery.

Do not delete the runtime state file to recover an instance. The UI actions preserve the diagnostic trail and coordinate correctly with the background runtime service.
