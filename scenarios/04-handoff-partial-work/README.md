# Handoff of half-finished work across agents

## Story

Agent A completes 3 of 5 subtasks for a rate-limiter feature, leaves
`test_burst_window` failing, writes a structured handoff (done / remaining /
next step / verification / files), then is killed mid-edit on
`limiter.py`.

Agent B on another machine resumes from the handoff.

## Success criteria

- B completes remaining work without redoing or reverting A's commits.
- Handoff record closed when the work item finishes; all 5 subtasks done, tests green.
