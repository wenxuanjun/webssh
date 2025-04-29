import { run } from './pkg'

function setupEventListeners() {
    const setListener = () => {
        const connectButton = document.getElementById('connect-button')
        if (connectButton) {
            connectButton.addEventListener('click', handleConnect)
        }
    }

    document.addEventListener('DOMContentLoaded', () => setListener())
    if (document.readyState === 'complete' || document.readyState === 'interactive') {
        setListener()
    }
  }

async function handleConnect() {
    const formElements = {
        wsAddress: document.getElementById('websocket-address'),
        sshAddress: document.getElementById('ssh-address'),
        sshPort: document.getElementById('ssh-port'),
        sshUsername: document.getElementById('ssh-username'),
        sshPassword: document.getElementById('ssh-password')
    }

    if (Object.values(formElements).some(el => !el)) {
        alert("Error: Could not find all required form elements")
        return
    }

    const values = Object.entries(formElements).reduce((acc, [key, el]) => {
        acc[key] = el.value.trim()
        return acc
    }, {})

    if (Object.values(values).some(val => !val)) {
        alert('Please fill in all fields')
        return
    }

    const sshAddress = encodeURIComponent(values.sshAddress)
    const sshPort = encodeURIComponent(values.sshPort)
    const wsAddressWithParams = `${values.wsAddress}?host=${sshAddress}&port=${sshPort}`

    try {
        await run(wsAddressWithParams, values.sshUsername, values.sshPassword)
    } catch (error) {
        console.error('Connection error:', error)
        alert(`Connection failed: ${error.message}`)
    }
}

setupEventListeners()
