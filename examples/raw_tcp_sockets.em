listener: TcpListener = TcpListener.bind("127.0.0.1", 47201)
client: TcpStream = TcpStream.connect("127.0.0.1", 47201)
client.write("ping")

server_side: TcpStream = listener.accept()
msg: String = server_side.read(64)
puts msg
server_side.write("pong")

reply: String = client.read(64)
puts reply

client.close()
server_side.close()
listener.close()
