ikm: String = "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"
salt: String = "000102030405060708090a0b0c"
info: String = "f0f1f2f3f4f5f6f7f8f9"

okm: String = Kdf.hkdf(ikm, salt, info, 42)
puts okm

pw_key: String = Kdf.pbkdf2("password", "73616c74", 600000, 20)
puts pw_key
