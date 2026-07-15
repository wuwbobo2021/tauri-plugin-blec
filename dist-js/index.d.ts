export type BleDevice = {
    address: string;
    name: string;
    rssi: number;
    isConnected: boolean;
    isBonded: boolean;
    services: string[];
    manufacturerData: Record<number, number[]>;
    serviceData: Record<string, number[]>;
    txPowerLevel?: number;
};
export type BleCharacteristic = {
    uuid: string;
    descriptors: string[];
    properties: number;
};
export type BleService = {
    uuid: string;
    characteristics: BleCharacteristic[];
};
export type AdapterState = "Unknown" | "On" | "Off";
/**
 * Get the current state of the BLE adapter (on/off)
 */
export declare function getAdapterState(): Promise<AdapterState>;
/**
 * Scan for BLE devices
 * @param handler - A function that will be called with an array of devices found during the scan
 * @param timeout - The scan timeout in milliseconds
 */
export declare function startScan(handler: (devices: BleDevice[]) => void, timeout: Number, allowIbeacons?: boolean): Promise<void>;
/**
 * Stop scanning for BLE devices
 */
export declare function stopScan(): Promise<void>;
/**
 * Check if necessary permissions are granted
 * @ param askIfDenied - If true, will ask the user for permissions again, if they were denied before
 * @returns true if permissions are granted, false otherwise
 */
export declare function checkPermissions(askIfDenied?: boolean): Promise<boolean>;
/**
 * Register a handler to receive updates when the connection state changes
 */
export declare function getConnectionUpdates(handler: (connected: boolean) => void): Promise<void>;
/**
 * Register a handler to receive updates when the scanning state changes
 */
export declare function getScanningUpdates(handler: (scanning: boolean) => void): Promise<void>;
/**
 * Disconnect from the currently connected device
 */
export declare function disconnect(): Promise<void>;
/**
 * Connect to a BLE device
 * @param address - The address of the device to connect to
 * @param onDisconnect - A function that will be called when the device disconnects
 */
export declare function connect(address: string, onDisconnect: (() => void) | null, allowIbeacons?: boolean): Promise<void>;
/**
 * Write a byte array to a BLE characteristic
 * @param characteristic UUID of the characteristic to write to
 * @param data Data to write to the characteristic
 */
export declare function send(characteristic: string, data: number[], writeType?: "withResponse" | "withoutResponse", service?: string): Promise<void>;
/**
 * Write a string to a BLE characteristic
 * @param characteristic UUID of the characteristic to write to
 * @param data Data to write to the characteristic
 */
export declare function sendString(characteristic: string, data: string, writeType?: "withResponse" | "withoutResponse", service?: string): Promise<void>;
/**
 * Read bytes from a BLE characteristic
 * @param characteristic UUID of the characteristic to read from
 */
export declare function read(characteristic: string, service?: string): Promise<number[]>;
/**
 * Read a string from a BLE characteristic
 * @param characteristic UUID of the characteristic to read from
 */
export declare function readString(characteristic: string, service?: string): Promise<string>;
/**
 * Unsubscribe from a BLE characteristic
 * @param characteristic UUID of the characteristic to unsubscribe from
 */
export declare function unsubscribe(characteristic: string, service?: string): Promise<void>;
/**
 * Subscribe to a BLE characteristic
 * @param characteristic UUID of the characteristic to subscribe to
 * @param handler Callback function that will be called with the data received for every notification
 */
export declare function subscribe(characteristic: string, service: string | null, handler: (data: number[]) => void): Promise<void>;
/**
 * Subscribe to a BLE characteristic. Converts the received data to a string
 * @param characteristic UUID of the characteristic to subscribe to
 * @param handler Callback function that will be called with the data received for every notification
 */
export declare function subscribeString(characteristic: string, service: string | null, handler: (data: string) => void): Promise<void>;
/**
 * List device services.
 */
export declare function listServices(address: string): Promise<BleService[] | string>;
