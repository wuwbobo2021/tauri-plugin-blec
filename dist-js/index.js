import { invoke, Channel } from '@tauri-apps/api/core';

/**
 * Get the current state of the BLE adapter (on/off)
 */
async function getAdapterState() {
    let state = await invoke("plugin:blec|get_adapter_state");
    return state;
}
/**
 * Scan for BLE devices
 * @param handler - A function that will be called with an array of devices found during the scan
 * @param timeout - The scan timeout in milliseconds
 */
async function startScan(handler, timeout, allowIbeacons = false) {
    if (!timeout) {
        timeout = 10000;
    }
    let onDevices = new Channel();
    onDevices.onmessage = handler;
    await invoke("plugin:blec|scan", {
        timeout,
        onDevices,
        allowIbeacons,
    });
}
/**
 * Stop scanning for BLE devices
 */
async function stopScan() {
    await invoke("plugin:blec|stop_scan");
}
/**
 * Check if necessary permissions are granted
 * @ param askIfDenied - If true, will ask the user for permissions again, if they were denied before
 * @returns true if permissions are granted, false otherwise
 */
async function checkPermissions(askIfDenied = true) {
    return await invoke("plugin:blec|check_permissions", { askIfDenied });
}
/**
 * Register a handler to receive updates when the connection state changes
 */
async function getConnectionUpdates(handler) {
    let connection_chan = new Channel();
    connection_chan.onmessage = handler;
    await invoke("plugin:blec|connection_state", { update: connection_chan });
}
/**
 * Register a handler to receive updates when the scanning state changes
 */
async function getScanningUpdates(handler) {
    let scanning_chan = new Channel();
    scanning_chan.onmessage = handler;
    await invoke("plugin:blec|scanning_state", { update: scanning_chan });
}
/**
 * Disconnect from the currently connected device
 */
async function disconnect() {
    await invoke("plugin:blec|disconnect");
}
/**
 * Connect to a BLE device
 * @param address - The address of the device to connect to
 * @param onDisconnect - A function that will be called when the device disconnects
 */
async function connect(address, onDisconnect, allowIbeacons = false) {
    let disconnectChannel = new Channel();
    if (onDisconnect) {
        disconnectChannel.onmessage = onDisconnect;
    }
    await invoke("plugin:blec|connect", {
        address: address,
        onDisconnect: disconnectChannel,
        allowIbeacons,
    });
}
/**
 * Write a byte array to a BLE characteristic
 * @param characteristic UUID of the characteristic to write to
 * @param data Data to write to the characteristic
 */
async function send(characteristic, data, writeType = "withResponse", service) {
    await invoke("plugin:blec|send", {
        characteristic,
        data,
        writeType,
        service,
    });
}
/**
 * Write a string to a BLE characteristic
 * @param characteristic UUID of the characteristic to write to
 * @param data Data to write to the characteristic
 */
async function sendString(characteristic, data, writeType = "withResponse", service) {
    await invoke("plugin:blec|send_string", {
        characteristic,
        data,
        writeType,
        service,
    });
}
/**
 * Read bytes from a BLE characteristic
 * @param characteristic UUID of the characteristic to read from
 */
async function read(characteristic, service) {
    let res = await invoke("plugin:blec|recv", {
        characteristic,
        service,
    });
    return res;
}
/**
 * Read a string from a BLE characteristic
 * @param characteristic UUID of the characteristic to read from
 */
async function readString(characteristic, service) {
    let res = await invoke("plugin:blec|recv_string", {
        characteristic,
        service,
    });
    return res;
}
/**
 * Unsubscribe from a BLE characteristic
 * @param characteristic UUID of the characteristic to unsubscribe from
 */
async function unsubscribe(characteristic, service) {
    await invoke("plugin:blec|unsubscribe", {
        characteristic,
        service
    });
}
/**
 * Subscribe to a BLE characteristic
 * @param characteristic UUID of the characteristic to subscribe to
 * @param handler Callback function that will be called with the data received for every notification
 */
async function subscribe(characteristic, service, handler) {
    let onData = new Channel();
    onData.onmessage = handler;
    await invoke("plugin:blec|subscribe", {
        characteristic,
        service,
        onData,
    });
}
/**
 * Subscribe to a BLE characteristic. Converts the received data to a string
 * @param characteristic UUID of the characteristic to subscribe to
 * @param handler Callback function that will be called with the data received for every notification
 */
async function subscribeString(characteristic, service, handler) {
    let onData = new Channel();
    onData.onmessage = handler;
    await invoke("plugin:blec|subscribe_string", {
        characteristic,
        service,
        onData,
    });
}
/**
 * List device services.
 */
async function listServices(address) {
    let res = await invoke("plugin:blec|list_services", {
        address: address,
    });
    return res;
}

export { checkPermissions, connect, disconnect, getAdapterState, getConnectionUpdates, getScanningUpdates, listServices, read, readString, send, sendString, startScan, stopScan, subscribe, subscribeString, unsubscribe };
