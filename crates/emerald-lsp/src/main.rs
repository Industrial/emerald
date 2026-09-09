use lsp_server::Connection;

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
  let (connection, io_threads) = Connection::stdio();
  emerald_lsp::run(&connection)?;
  io_threads.join()?;
  Ok(())
}
