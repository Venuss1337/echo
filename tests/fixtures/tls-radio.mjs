import { readFileSync } from "node:fs";
import { createServer } from "node:tls";

const [pfx, redirectPort] = process.argv.slice(2);
const response =
  redirectPort !== "0"
    ? `HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:${redirectPort}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n`
    : "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc";

const server = createServer(
  { pfx: readFileSync(pfx), passphrase: "" },
  (socket) => {
    socket.setTimeout(4000, () => socket.destroy());
    socket.once("data", () => socket.end(response));
    socket.on("error", () => socket.destroy());
    server.close();
  },
);
const deadline = setTimeout(() => {
  server.close();
  process.exitCode = 1;
}, 7000);
server.on("close", () => clearTimeout(deadline));
server.on("tlsClientError", () => server.close());
server.listen(0, "127.0.0.1", () => console.log(server.address().port));
