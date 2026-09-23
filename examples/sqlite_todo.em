# Plan 137 (SQLite): `Sqlite.open_memory`/`.execute_direct`/`.prepare`/
# `.bind_string`/`.bind_int64`/`.execute`/`.query`/`.step`/
# `.column_int64`/`.column_string`/`.close` — an in-memory database
# created, a row inserted through a bound parameter (never a
# formatted/interpolated string), and read back through a prepared,
# bound `SELECT`, proving the parameterized-only path round-trips
# correctly end to end with no file left behind on disk (`:memory:`,
# never a real path). Matches this plan's own Concrete Proof verbatim.

db: Int64 = Sqlite.open_memory()
Sqlite.execute_direct(db, "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL)")

insert: Int64 = Sqlite.prepare(db, "INSERT INTO todos (title, done) VALUES (?, ?)")
Sqlite.bind_string(insert, 1, "write plan 137")
Sqlite.bind_int64(insert, 2, 0)
Sqlite.execute(insert)

select: Int64 = Sqlite.prepare(db, "SELECT id, title FROM todos WHERE done = ?")
Sqlite.bind_int64(select, 1, 0)
cursor: Int64 = Sqlite.query(select)
while Sqlite.step(cursor) do
  id: Int64 = Sqlite.column_int64(cursor, 0)
  title: String = Sqlite.column_string(cursor, 1)
  puts id
  puts title
end
Sqlite.close(db)
