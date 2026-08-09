import fs from "node:fs";

const asBigInt = (value) => typeof value === "bigint" ? value : BigInt(value);

export class SplitScriptHost {
    constructor({
        settings = {},
        operatingSystem = "windows",
        architecture = "x86_64",
        timerState = 0,
        currentSplitIndex = null,
        clockNanoseconds = 0n,
    } = {}) {
        this.decoder = new TextDecoder();
        this.encoder = new TextEncoder();
        this.instance = undefined;

        this.operatingSystem = operatingSystem;
        this.architecture = architecture;
        this.timerState = timerState;
        this.currentSplitIndex = currentSplitIndex;
        this.segmentHistory = new Map();
        this.clockNanoseconds = asBigInt(clockNanoseconds);

        this.settings = new Map(Object.entries(settings));
        this.settingsMaps = new Map();
        this.settingValues = new Map();
        this.nextSettingsMapHandle = 1n;
        this.nextSettingValueHandle = 1n;

        this.processes = new Map();
        this.processHandles = new Map();
        this.nextProcessHandle = 1n;

        this.widgets = [];
        this.filters = [];
        this.tooltips = new Map();
        this.variables = new Map();
        this.messages = [];
        this.tickRates = [];
        this.attachAttempts = [];
        this.detaches = [];
        this.timerCalls = {
            starts: 0,
            splits: 0,
            skips: 0,
            undos: 0,
            resets: 0,
            gameTimes: [],
            pauses: 0,
            resumes: 0,
        };

        this.imports = {
            env: this.createEnvironmentImports(),
            wasi_snapshot_preview1: {
                clock_time_get: (_clock, _precision, outputPointer) => {
                    this.view().setBigUint64(outputPointer, this.clockNanoseconds, true);
                    return 0;
                },
            },
        };
    }

    static async instantiate(wasmPath, options) {
        const host = new SplitScriptHost(options);
        const bytes = fs.readFileSync(wasmPath);
        ({ instance: host.instance } = await WebAssembly.instantiate(bytes, host.imports));
        return host;
    }

    addProcess(name, {
        open = true,
        path = name,
        modules = {},
        ranges = [],
        read = undefined,
    } = {}) {
        const normalizedModules = new Map(
            Object.entries(modules).map(([moduleName, module]) => [
                moduleName,
                {
                    address: asBigInt(module.address),
                    size: asBigInt(module.size),
                    path: module.path ?? moduleName,
                },
            ]),
        );
        const normalizedRanges = ranges.map((range) => ({
            address: asBigInt(range.address),
            bytes: range.bytes instanceof Uint8Array
                ? range.bytes
                : Uint8Array.from(range.bytes),
            flags: asBigInt(range.flags ?? 0),
        }));
        const process = {
            name,
            open,
            path,
            modules: normalizedModules,
            ranges: normalizedRanges,
            read,
        };
        this.processes.set(name, process);
        return process;
    }

    setProcessOpen(name, open) {
        this.process(name).open = open;
    }

    setSetting(key, value) {
        this.settings.set(key, value);
    }

    start() {
        this.instance.exports._start();
    }

    update(count = 1) {
        for (let index = 0; index < count; index += 1) {
            this.instance.exports.update();
        }
    }

    updateUntil(condition, description, limit = 32) {
        for (let tick = 0; tick < limit && !condition(); tick += 1) {
            this.update();
        }
        if (!condition()) {
            throw new Error(`${description}: ${this.json(this.summary())}`);
        }
    }

    json(value) {
        return JSON.stringify(value, (_key, item) => (
            typeof item === "bigint" ? item.toString() : item
        ));
    }

    summary() {
        return {
            tickRates: this.tickRates,
            attachAttempts: this.attachAttempts,
            detaches: this.detaches,
            timerCalls: this.timerCalls,
            messages: this.messages,
            variables: Object.fromEntries(this.variables),
        };
    }

    createEnvironmentImports() {
        return {
            timer_get_state: () => this.timerState,
            timer_current_split_index: () => this.currentSplitIndex === null
                ? -1n
                : asBigInt(this.currentSplitIndex),
            timer_segment_splitted: (index) => this.segmentHistory.get(asBigInt(index)) ?? -1,
            timer_start: () => {
                this.timerCalls.starts += 1;
                this.timerState = 1;
            },
            timer_split: () => { this.timerCalls.splits += 1; },
            timer_skip_split: () => { this.timerCalls.skips += 1; },
            timer_undo_split: () => { this.timerCalls.undos += 1; },
            timer_reset: () => {
                this.timerCalls.resets += 1;
                this.timerState = 0;
            },
            timer_set_game_time: (seconds, nanoseconds) => {
                this.timerCalls.gameTimes.push([seconds, nanoseconds]);
            },
            timer_pause_game_time: () => { this.timerCalls.pauses += 1; },
            timer_resume_game_time: () => { this.timerCalls.resumes += 1; },
            timer_set_variable: (keyPointer, keyLength, valuePointer, valueLength) => {
                this.variables.set(
                    this.text(keyPointer, keyLength),
                    this.text(valuePointer, valueLength),
                );
            },
            runtime_set_tick_rate: (rate) => { this.tickRates.push(rate); },
            runtime_print_message: (pointer, length) => {
                this.messages.push(this.text(pointer, length));
            },
            runtime_get_os: (pointer, lengthPointer) => this.provideText(
                this.operatingSystem,
                pointer,
                lengthPointer,
            ),
            runtime_get_arch: (pointer, lengthPointer) => this.provideText(
                this.architecture,
                pointer,
                lengthPointer,
            ),
            process_attach: (pointer, length) => {
                const name = this.text(pointer, length);
                this.attachAttempts.push(name);
                const process = this.processes.get(name);
                if (!process?.open) return 0n;
                const handle = this.nextProcessHandle++;
                this.processHandles.set(handle, process);
                return handle;
            },
            process_detach: (handle) => {
                this.detaches.push(handle);
                this.processHandles.delete(handle);
            },
            process_is_open: (handle) => this.attachedProcess(handle).open ? 1 : 0,
            process_read: (handle, address, outputPointer, length) => {
                const process = this.attachedProcess(handle);
                const target = asBigInt(address);
                if (process.read) {
                    return process.read({
                        address: target,
                        outputPointer,
                        length,
                        host: this,
                    }) ? 1 : 0;
                }
                const end = target + BigInt(length);
                const range = process.ranges.find((candidate) => {
                    const rangeEnd = candidate.address + BigInt(candidate.bytes.length);
                    return target >= candidate.address && end <= rangeEnd;
                });
                if (!range) return 0;
                const offset = Number(target - range.address);
                this.bytes(outputPointer, length).set(range.bytes.subarray(offset, offset + length));
                return 1;
            },
            process_get_module_address: (handle, pointer, length) => {
                const module = this.module(handle, this.text(pointer, length));
                return module?.address ?? 0n;
            },
            process_get_module_size: (handle, pointer, length) => {
                const module = this.module(handle, this.text(pointer, length));
                return module?.size ?? 0n;
            },
            process_get_module_path: (handle, namePointer, nameLength, pathPointer, lengthPointer) => {
                const module = this.module(handle, this.text(namePointer, nameLength));
                return module ? this.provideText(module.path, pathPointer, lengthPointer) : 0;
            },
            process_get_path: (handle, pointer, lengthPointer) => this.provideText(
                this.attachedProcess(handle).path,
                pointer,
                lengthPointer,
            ),
            process_get_memory_range_count: (handle) => BigInt(
                this.attachedProcess(handle).ranges.length,
            ),
            process_get_memory_range_address: (handle, index) => this.memoryRange(handle, index).address,
            process_get_memory_range_size: (handle, index) => BigInt(
                this.memoryRange(handle, index).bytes.length,
            ),
            process_get_memory_range_flags: (handle, index) => this.memoryRange(handle, index).flags,
            user_settings_add_bool: (
                keyPointer,
                keyLength,
                descriptionPointer,
                descriptionLength,
                defaultValue,
            ) => {
                const key = this.text(keyPointer, keyLength);
                if (!this.settings.has(key)) this.settings.set(key, defaultValue !== 0);
                this.widgets.push([
                    "bool",
                    key,
                    this.text(descriptionPointer, descriptionLength),
                ]);
                return this.settings.get(key) === true ? 1 : 0;
            },
            user_settings_add_title: (
                keyPointer,
                keyLength,
                descriptionPointer,
                descriptionLength,
                level,
            ) => {
                this.widgets.push([
                    "title",
                    this.text(keyPointer, keyLength),
                    this.text(descriptionPointer, descriptionLength),
                    level,
                ]);
            },
            user_settings_add_choice: (
                keyPointer,
                keyLength,
                descriptionPointer,
                descriptionLength,
                defaultPointer,
                defaultLength,
            ) => {
                const key = this.text(keyPointer, keyLength);
                if (!this.settings.has(key)) {
                    this.settings.set(key, this.text(defaultPointer, defaultLength));
                }
                this.widgets.push([
                    "choice",
                    key,
                    this.text(descriptionPointer, descriptionLength),
                ]);
            },
            user_settings_add_choice_option: (
                keyPointer,
                keyLength,
                optionPointer,
                optionLength,
                descriptionPointer,
                descriptionLength,
            ) => {
                const key = this.text(keyPointer, keyLength);
                const option = this.text(optionPointer, optionLength);
                this.widgets.push([
                    "option",
                    key,
                    option,
                    this.text(descriptionPointer, descriptionLength),
                ]);
                return this.settings.get(key) === option ? 1 : 0;
            },
            user_settings_add_file_select: (
                keyPointer,
                keyLength,
                descriptionPointer,
                descriptionLength,
            ) => {
                const key = this.text(keyPointer, keyLength);
                if (!this.settings.has(key)) this.settings.set(key, "");
                this.widgets.push([
                    "file",
                    key,
                    this.text(descriptionPointer, descriptionLength),
                ]);
            },
            user_settings_add_file_select_name_filter: (
                keyPointer,
                keyLength,
                descriptionPointer,
                descriptionLength,
                patternPointer,
                patternLength,
            ) => {
                this.filters.push([
                    "name",
                    this.text(keyPointer, keyLength),
                    descriptionPointer === 0
                        ? null
                        : this.text(descriptionPointer, descriptionLength),
                    this.text(patternPointer, patternLength),
                ]);
            },
            user_settings_add_file_select_mime_filter: (
                keyPointer,
                keyLength,
                mimePointer,
                mimeLength,
            ) => {
                this.filters.push([
                    "mime",
                    this.text(keyPointer, keyLength),
                    this.text(mimePointer, mimeLength),
                ]);
            },
            user_settings_set_tooltip: (
                keyPointer,
                keyLength,
                tooltipPointer,
                tooltipLength,
            ) => {
                this.tooltips.set(
                    this.text(keyPointer, keyLength),
                    this.text(tooltipPointer, tooltipLength),
                );
            },
            settings_map_load: () => {
                const handle = this.nextSettingsMapHandle++;
                this.settingsMaps.set(handle, new Map(this.settings));
                return handle;
            },
            settings_map_free: (handle) => { this.settingsMaps.delete(handle); },
            settings_map_get: (mapHandle, keyPointer, keyLength) => {
                const values = this.settingsMaps.get(mapHandle);
                const key = this.text(keyPointer, keyLength);
                if (!values?.has(key)) return 0n;
                const handle = this.nextSettingValueHandle++;
                this.settingValues.set(handle, values.get(key));
                return handle;
            },
            setting_value_free: (handle) => { this.settingValues.delete(handle); },
            setting_value_get_bool: (handle, outputPointer) => {
                const value = this.settingValues.get(handle);
                if (typeof value !== "boolean") return 0;
                this.view().setUint8(outputPointer, value ? 1 : 0);
                return 1;
            },
            setting_value_get_string: (handle, outputPointer, lengthPointer) => {
                const value = this.settingValues.get(handle);
                if (typeof value !== "string") {
                    this.view().setUint32(lengthPointer, 0, true);
                    return 0;
                }
                return this.provideText(value, outputPointer, lengthPointer);
            },
        };
    }

    process(name) {
        const process = this.processes.get(name);
        if (!process) throw new Error(`unknown fixture process ${JSON.stringify(name)}`);
        return process;
    }

    attachedProcess(handle) {
        const process = this.processHandles.get(handle);
        if (!process) throw new Error(`unknown process handle ${handle}`);
        return process;
    }

    module(handle, name) {
        return this.attachedProcess(handle).modules.get(name);
    }

    memoryRange(handle, index) {
        const range = this.attachedProcess(handle).ranges[Number(index)];
        if (!range) throw new Error(`unknown memory range ${index}`);
        return range;
    }

    view() {
        return new DataView(this.instance.exports.memory.buffer);
    }

    bytes(pointer, length) {
        return new Uint8Array(this.instance.exports.memory.buffer, pointer, length);
    }

    text(pointer, length) {
        return this.decoder.decode(this.bytes(pointer, length));
    }

    provideText(value, pointer, lengthPointer) {
        const encoded = this.encoder.encode(value);
        const view = this.view();
        const capacity = view.getUint32(lengthPointer, true);
        view.setUint32(lengthPointer, encoded.length, true);
        if (capacity < encoded.length) return 0;
        this.bytes(pointer, encoded.length).set(encoded);
        return 1;
    }
}
