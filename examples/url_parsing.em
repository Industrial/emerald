# Plan 98 (URL Parsing) — `Url.parse`/`.build`/`#scheme`/`#host`/
# `#port`/`#path`/`#query`/`#fragment`/`#with_query`, wrapping the
# `url` crate (Servo's WHATWG-Standard-compliant reference
# implementation — the crate the rest of the Rust web ecosystem
# already depends on). Plans 100/101 (HTTP client/server) both build
# on this one already-correct implementation rather than each hand-
# rolling string-splitting logic.
#
# `plain.port` (`443` from a URL with no explicit port at all) is the
# real proof of `#port`'s `port_or_known_default()`-backed scheme-
# default fallback — a bare string-splitting implementation would
# naturally return an empty/absent port here.
#
# Real, disclosed note: plan 96 (raw TCP/UDP sockets), which this
# plan's own text cites for its handle-representation convention, was
# not executed before this plan — `Url` instead reuses this session's
# own already-established `Int64`-newtype-over-handle-registry shape.

u: Url = Url.parse("https://example.com/search?q=emerald#results")
puts u.scheme()
puts u.host()
puts u.port()
puts u.path()
puts u.query()
puts u.fragment()

plain: Url = Url.parse("https://example.com")
puts plain.port()

built: Url = Url.build("https", "example.com", "/api")
q: Url = built.with_query("id=42")
puts q.path()
puts q.query()
