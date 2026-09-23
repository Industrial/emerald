# Plan 142 (Embedded ACID Database, redb): `Redb.open`/`.table`/
# `.begin_write`/`.table_insert`/`.commit`/`.begin_read`/`.table_get`/
# `.close` -- two rows written and committed in one write transaction,
# then read back through a separate read transaction, with a genuine
# miss defaulted via `match`/`Some`/`None`.
#
# Real, disclosed correction: this plan's own Concrete Proof text used
# `String?` (`Redb.table_get`'s return type) and the `||=` operator to
# default a miss -- that nullable sugar and `||=` were both removed
# outright (plan 73, the same correction plan 146's own
# `environment_variables_proof.em` already discloses for `Env.get`);
# the real, current annotation is `Option[String]`, unwrapped via
# `match`/`Some`/`None`.

db: Int64 = Redb.open("plan142_demo.redb")
users: Int64 = Redb.table("users")

write_txn: Int64 = Redb.begin_write(db)
Redb.table_insert(write_txn, users, "1", "Ada")
Redb.table_insert(write_txn, users, "2", "Grace")
Redb.commit(write_txn)

read_txn: Int64 = Redb.begin_read(db)

first: Option[String] = Redb.table_get(read_txn, users, "1")
match first do
Some(v) do
  puts v
end
None do
  puts "unknown"
end
end

second: Option[String] = Redb.table_get(read_txn, users, "2")
match second do
Some(v) do
  puts v
end
None do
  puts "unknown"
end
end

missing: Option[String] = Redb.table_get(read_txn, users, "3")
match missing do
Some(v) do
  puts v
end
None do
  puts "not found"
end
end

Redb.close(db)
