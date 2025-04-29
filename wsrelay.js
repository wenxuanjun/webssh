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

// const WebSocket = require('ws');
// const net = require('node:net');

// const WS_HOST = process.env.WS_HOST || "0.0.0.0";
// const WS_PORT = process.env.WS_PORT || 19198;
// const TCP_HOST = process.env.TCP_HOST || "localhost";
// const TCP_PORT = process.env.TCP_PORT || 22;

// const wss = new WebSocket.Server({ host: WS_HOST, port: WS_PORT });

// wss.on('connection', (ws) => {
//     console.log('New WebSocket connection');

//     const tcpSocket = net.createConnection({ host: TCP_HOST, port: TCP_PORT }, () => {
//         console.log('Connected to TCP server');
//     });

//     tcpSocket.on('data', (data) => {
//         ws.send(data);
//     });

//     tcpSocket.on('error', (err) => {
//         console.error('TCP Error:', err);
//         ws.close();
//     });

//     tcpSocket.on('close', () => {
//         console.log('TCP connection closed');
//         ws.close();
//     });

//     ws.on('message', (message) => {
//         tcpSocket.write(message);
//     });

//     ws.on('close', () => {
//         console.log('WebSocket connection closed');
//         tcpSocket.end();
//     });

//     ws.on('error', (err) => {
//         console.error('WebSocket Error:', err);
//         tcpSocket.end();
//     });
// });

// console.log('Listening on ws://%s:%d', WS_HOST, WS_PORT);
