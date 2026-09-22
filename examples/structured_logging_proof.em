# Plan 168 (Structured Logging) — a compiler-provided `Log` module
# (`Log.trace`/`.debug`/`.info`/`.warn`/`.error`, each with a
# `Log.<level>_fields` sibling taking a `LogFields` builder handle)
# backed by `tracing`/`tracing-subscriber`. `LogFields` reuses plan
# 93's own resource-handle registry rather than a bespoke mechanism,
# since plan 93 landed earlier this same session.
#
# All output here is on stderr, deliberately kept structurally
# separate from stdout (which only ever carries `puts`) — this
# example's own CLI test asserts against stderr, not stdout.
Log.configure("info", "json")

Log.info("service starting")

fields: LogFields = LogFields.new()
fields.set("user_id", "42")
fields.set("plan", "pro")
Log.info_fields("user signed in", fields)

Log.warn("cache miss")
Log.debug("this line is below the configured level and prints nothing")
