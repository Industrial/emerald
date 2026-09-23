# Plan 112 (Password Hashing) — `Password.hash`/`.verify`, wrapping
# `argon2` (RustCrypto, defaulting to the Argon2id variant). Salt
# generation is automatic and internal — `Password.hash`'s own PHC-
# string output is salted and non-deterministic by design (hashing
# the same plaintext twice produces two different strings), so no
# fixed hash value is asserted here, only round-trip correctness —
# the same honest constraint this plan's own Concrete Proof states.
#
# Real, disclosed correction: `puts` doesn't accept a bare `Boolean` —
# string interpolation (`"#{...}"`, which does support it) is used
# instead, the same workaround established throughout this session
# (see `secure_compare_proof.em`, `random_csprng_proof.em`).

password: String = "correct horse battery staple"
hash: String = Password.hash(password)

correct_result: Boolean = Password.verify(password, hash)
wrong_result: Boolean = Password.verify("wrong password", hash)
has_length: Boolean = hash.length > 0

puts "#{correct_result}"
puts "#{wrong_result}"
puts "#{has_length}"
