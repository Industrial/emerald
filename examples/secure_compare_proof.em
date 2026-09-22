# Plan 117 (Constant-Time Comparison) — `SecureCompare.eq`, wrapping
# `subtle::ConstantTimeEq`. The comparison itself takes the same time
# regardless of where two equal-length inputs first differ (a
# property of `subtle`'s own implementation, verified by its own
# upstream test suite — not independently re-verified by this
# example, which can only check the return VALUE, not the timing
# property, exactly as this plan's own Decision log states). A
# differing-length pair is rejected via a real, disclosed short-
# circuit (`subtle`'s own documented behavior, not a bug) — accepted
# because every real caller in this batch compares fixed/public-length
# secrets, where length itself isn't the secret.
#
# Real, disclosed correction: `puts` doesn't accept a bare `Boolean` —
# string interpolation (`"#{...}"`, which does support it) is used
# instead, the same workaround established throughout this session.

secret: String = "s3cr3t-api-key-do-not-leak"
same_value: String = "s3cr3t-api-key-do-not-leak"
different_value: String = "s3cr3t-api-key-do-not-leek"
short_value: String = "too-short"

same_result: Boolean = SecureCompare.eq(secret, same_value)
different_result: Boolean = SecureCompare.eq(secret, different_value)
short_result: Boolean = SecureCompare.eq(secret, short_value)
puts "#{same_result}"
puts "#{different_result}"
puts "#{short_result}"
