# Plan 184 (TOTP/HOTP Two-Factor Authentication) — `Totp.new`,
# `Totp#generate_current`/`#check_current`/`#provisioning_uri`,
# wrapping `totp-rs`. SHA1, 6 digits, 30-second step, 1-step skew —
# `totp_rs::Builder::new()`'s own real, unchanged defaults, matching
# what every real authenticator app (Google Authenticator, Authy)
# actually expects (this plan's own Decision log).
#
# Real, disclosed deviations from this plan's own literal Concrete
# Proof text: `Totp.new` returns `Result[Totp, TotpError]`, not a bare
# `Totp` — plan 195's Typed Domain Errors convention landed after this
# plan was authored but before it was executed, so this example
# `match`es the result the same "lands after 195, retrofit directly"
# way every other post-195 fallible constructor's own example already
# does (see `crates/emerald-rt/src/totp.rs`'s own module doc for the
# full account). The plan's own literal secret,
# `"JBSWY3DPEHPK3PXP"`, base32-decodes to only 80 bits — below the
# 128-bit minimum `totp_rs` itself enforces (`Builder::build`, the
# real RFC-compliant path this module uses, not the RFC-skipping
# `build_noncompliant`) — so this example doubles it
# (`"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"`, 160 bits) rather than
# silently accepting an under-strength secret.
#
# `code`'s own value is genuinely time-dependent (the current
# wall-clock 30-second step) — this example's own compiled-and-run
# test harness (`emerald-cli/tests/examples.rs`) checks only that it
# is six ASCII digits and that `check_current` immediately accepts it,
# never a fixed literal, exactly as this plan's own Concrete Proof
# describes. `valid`/`invalid`/`uri` are all genuinely deterministic
# given these fixed inputs and are checked exactly. A third real,
# disclosed correction, the same one nearly every other proof example
# in this batch already makes: `puts` doesn't accept a bare `Boolean`
# directly, needing string interpolation instead.

result: Result[Totp, TotpError] = Totp.new("Emerald Corp", "alice@example.com", "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP")
match result do
  Ok(totp) do
    code: String = totp.generate_current()
    puts code

    valid: Boolean = totp.check_current(code)
    puts "#{valid}"

    invalid: Boolean = totp.check_current("000000")
    puts "#{invalid}"

    uri: String = totp.provisioning_uri()
    puts uri
  end
  Err(e) do
    puts "unexpected error"
  end
end
