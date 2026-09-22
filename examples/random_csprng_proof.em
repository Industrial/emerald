# Plan 113 (Cryptographically Secure Random Number Generation) —
# `Random.secure_hex` (CSPRNG-backed, safe for keys/tokens/nonces) and
# `Random.int`/`.shuffle` (fast, non-cryptographic, never for
# secrets) — deliberately separate top-level names, not one generator
# behind a boolean flag, per this plan's own Decision log.
#
# `token`'s own literal value is never asserted — it must differ on
# every run by design; only its length is checked. Real, disclosed
# correction: `puts` doesn't accept a bare `Boolean` — string
# interpolation used instead, the same workaround established
# throughout this session.

token: String = Random.secure_hex(16)
puts token.length

low: Int64 = 1
high: Int64 = 6
roll: Int64 = Random.int(low, high)
in_range: Boolean = roll >= low && roll <= high
puts "#{in_range}"

a: Array[Int64] = [1, 2, 3, 4, 5]
Random.shuffle(a)
first: Int64 = a[0]
still_valid: Boolean = first >= 1 && first <= 5
puts "#{still_valid}"
