# Repro Bundle Handoff Plan (End-to-End)

## 1) Start Conduit in record mode (fresh data dir)

```bash
CONDUIT_REPRO_MODE=record conduit --data-dir /tmp/conduit-repro
```

## 2) Reproduce the bug in the UI

- Use the app normally until the bug appears.
- Stop any active run so the DB is idle.

## 3) Export the repro bundle

```bash
conduit repro export --out /tmp/conduit-repro.zip --mode local
```

## 4) (If workspace changed) capture a patch

```bash
git -C /path/to/workspace diff > /tmp/workspace.patch
```

## 5) Send the bundle + notes

- `/tmp/conduit-repro.zip`
- `/tmp/workspace.patch` (if applicable)
- A short note: what you did + what broke

---

## Replay (for reference)

### Quick replay (read-only)

```bash
conduit repro run /tmp/conduit-repro.zip --ui web --host 0.0.0.0
```

### Persistent replay

```bash
conduit repro extract /tmp/conduit-repro.zip --out-dir ./repro-data --overwrite
CONDUIT_REPRO_MODE=replay conduit --data-dir ./repro-data
```
