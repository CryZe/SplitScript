export const nativeBridgeFileName = 'splitscript_process_native.node';

export const nativeBridgeLibraries = Object.freeze({
    'win32-x64': 'splitscript_process_native.dll',
    'linux-x64': 'libsplitscript_process_native.so',
    'linux-arm64': 'libsplitscript_process_native.so',
    'darwin-x64': 'libsplitscript_process_native.dylib',
    'darwin-arm64': 'libsplitscript_process_native.dylib',
});

export const supportedNativePlatforms = Object.freeze(Object.keys(nativeBridgeLibraries));
