2026-09-23T06:00:00Z

# Plan 112 — Password Hashing — Update (2026-09-23, same-day session)

This is an update record for `history/2026-09-21T202100Z-plan-112-password-hashing.md`
(the original plan text, authored 2026-09-21) — kept as a separate,
dated file per this batch's own append-only convention rather than
editing the original, so the original's own Decision log stands
unmodified as written. Read that file first for the full original
design; this file records what was actually implemented and exactly
how it diverged.

## Status: implemented, all five leaves done

`argon2` 0.6.0 (RustCrypto, `password-hashes` project) plus its
`password-hash` feature (pulling in `password-hash` 0.6.1 and `phc`
0.6.1 transitively) added to `crates/emerald-rt/Cargo.toml`, row added
to `DEPENDENCIES.md` with the real transitive pulls (`base64ct`,
`blake2` 0.11, `cpufeatures`) verified directly against the installed
crate's own vendored source, not assumed from the plan's own text.

## The one real, disclosed API correction: `argon2` 0.6's `PasswordHasher` trait shape

The original plan's own text describes `PasswordHasher::hash_password`
taking an explicit `&SaltString` (`SaltString::generate(&mut OsRng)`,
constructed by this plan's own Rust code) as its second argument — the
API shape earlier `argon2`/`password-hash` releases actually had.
Checked directly against the installed `argon2` 0.6.0 / `password-hash`
0.6.1 source (not assumed from the plan's own confident phrasing, the
same discipline plan 123's own update applied to `Bytes`): the trait
has genuinely changed shape. `PasswordHasher::hash_password(&self,
password: &[u8]) -> Result<H>` is now a **single-argument** default
trait method (gated on the crate's own `getrandom` feature, on by
default) that generates a fresh random salt internally via
`password_hash::rand_core`-free internal
`getrandom`-backed generation — there is no `SaltString`/`OsRng`
import anywhere in the final code at all, an even stronger version of
the original plan's own "no Emerald-visible salt parameter" design
goal than the plan's own text anticipated, since the salt is no longer
externally constructible even on the Rust side. Separately, `argon2`'s
re-exported `password_hash::PasswordHash` type alias is deprecated in
0.6 in favor of `password_hash::phc::PasswordHash` (the crate's own
`#[deprecated]` attribute, not a guess) — this plan's code imports the
non-deprecated path directly. Both corrections were found by trying to
compile the plan's own literal text first (`cargo build -p
emerald-rt`), not by pre-emptively guessing the real API — the
resulting two compile errors named the exact fix (`E0061` too many
arguments to `hash_password`; `E0432` unresolved `rand_core` import
gated behind a feature this plan's own `Cargo.toml` line never
requested). The underlying design (Argon2id, OWASP's `m=19456,t=2,p=1`
tier, automatic/internal salting, `Password.verify`'s collapsed-to-
`Boolean` failure modes) is completely unaffected — only the Rust-side
call shape changed.

## Implementation

`crates/emerald-rt/src/password.rs` (new module): `password_hash`/
`password_verify`, the plain-`String`-in/`String`-or-`Boolean`-out
shape plan 115's `kdf.rs` already established for this compiler (no
`Bytes` type involvement — `Password.hash`/`.verify` operate on a
human password and a PHC string, both genuinely textual, never
arbitrary binary data). `Argon2::new(Algorithm::default(),
Version::default(), owasp_params())` backs `.hash`; `Argon2::default()
.verify_password` backs `.verify`, matching the crate's own doc
example exactly (`argon2-0.6.0/src/lib.rs`'s own usage doctest,
read directly rather than assumed). 5 `#[test]`s: round-trip,
wrong-password rejection, hand-corrupted-PHC-string rejection (both as
a real `false`, never a panic or propagated error), a completely
malformed `stored_hash` string, and a same-plaintext-twice-produces-
different-hashes salting proof.

`crates/emerald-rt/src/lib.rs`: `mod password;` plus two `#[no_mangle]
pub unsafe extern "C" fn emerald_rt_password_hash`/
`emerald_rt_password_verify`, each `catch_and_raise`-wrapped per plan
91's convention, dispatched by exact free-function name exactly like
`Kdf`/`SecureCompare` immediately beside them.

`crates/emerald-sema/src/lib.rs`: one new `Expr::MethodCall` arm
(`Password`), the identical reserved-namespace static-call shape
`SecureCompare`/`Kdf` already use, checked immediately after
`SecureCompare`'s own arm. `.hash: (String) -> String`, `.verify:
(String, String) -> Boolean`.

`crates/emerald-codegen/src/lib.rs`: matching `build_method_call`
dispatch (2 new `Ctx` fields, 2 new `module.add_function`
declarations, one new `if recv_name == "Password"` block) — `.verify`'s
`i64` 0/1 return is turned into a real `Boolean` via the identical
`build_int_compare` pattern `SecureCompare.eq`'s own dispatch arm
already established immediately above it. No codegen-level surprises;
the same `Kdf`/`SecureCompare` pattern copied, not a new mechanism.

## Concrete proof, verified end to end

`examples/password_hashing_proof.em` — the original plan's own
Concrete Proof, verbatim in structure: hashes a password, verifies it
against both the correct and a wrong plaintext, and checks the hash
string is non-empty. Output `true\nfalse\ntrue\n`, matching the
original plan's own predicted sequence exactly. No fixed hash string
is asserted, per the plan's own Decision log — `Password.hash`'s
output is salted and non-deterministic by design (proven directly by
this module's own `two_hashes_of_the_same_plaintext_are_different_
strings` Rust test, not merely assumed from the crate's docs).

The same real, disclosed `puts`-doesn't-accept-a-bare-`Boolean`
correction every other proof this session found applies here too —
worked around via string interpolation (`"#{...}"`), matching
`secure_compare_proof.em`/`random_csprng_proof.em`'s own established
pattern.

## Gate

Full workspace build (`cargo build --workspace`), `cargo test -p
emerald-rt password::` (5/5 passing), `cargo test -p emerald-cli
--test examples password_hashing` (1/1 passing — the compiled `.em`
proof above), `cargo clippy -p emerald-rt -p emerald-sema -p
emerald-codegen --all-targets` (clean, zero warnings against this
plan's own new code), `treefmt` (0 files changed) — all run against
an isolated `git worktree` checkout of this session's own `HEAD`
(a genuinely concurrent, multi-agent-committing shared working tree
this session — see this update's own git history for the interleaved
plan 160/193/99 commits landing during this plan's own implementation)
before the verified result was staged directly into the shared
repository's index via `git update-index --cacheinfo` per path, never
by editing a working-tree copy another agent might have been
concurrently writing to.

## Explicitly out of scope, unchanged from the original plan

Everything the original plan's own "Out of scope" bullet already
named (no configurable pepper, no password-strength validation, no
`Password.needs_rehash` migration helper) still applies unchanged —
no scope was added or removed by this update beyond the API-shape
correction documented above.
