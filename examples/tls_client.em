conn: TlsStream = Tls.connect("example.com", 443)
conn.write("GET / HTTP/1.1\nHost: example.com\nConnection: close\n\n")
status_line: String = conn.read(64)
puts status_line
conn.close()
