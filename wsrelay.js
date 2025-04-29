const WebSocket = require('ws');
const net = require('node:net');
const url = require('url');

const WS_HOST = process.env.WS_HOST || "0.0.0.0";
const WS_PORT = process.env.WS_PORT || 19198;

new WebSocket.Server({ host: WS_HOST, port: WS_PORT })
    .on('connection', (ws, req) => {
        const { remoteAddress = 'unknown', remotePort = 'unknown' } = req.socket;
        console.log('New connection from %s:%d', remoteAddress, remotePort);

        const { host: tcpHost, port } = url.parse(req.url, true).query;
        const tcpPort = port ? parseInt(port) : null;

        if (!tcpHost || !tcpPort) {
            console.error('Connection closed due to missing parameters');
            ws.close(1008, 'Missing required parameters: host, port');
            return;
        }

        const tcpSocket = net.createConnection({ host: tcpHost, port: tcpPort }, () =>
            console.log(`Connected to TCP server at ${tcpHost}:${tcpPort}`));

        tcpSocket.on('data', data => ws.send(data));
        tcpSocket.on('close', () => ws.close());
        tcpSocket.on('error', err => {
            console.error('TCP Error:', err);
            ws.close();
        });

        ws.on('message', data => tcpSocket.write(data));
        ws.on('close', () => tcpSocket.end());
        ws.on('error', err => {
            console.error('WebSocket Error:', err);
            tcpSocket.end();
        });
    })
    .on('listening', () => console.log(`Listening on ws://${WS_HOST}:${WS_PORT}`));
