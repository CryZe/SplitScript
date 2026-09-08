import type {
    SettingMapSnapshot,
    SettingsSnapshot,
    SettingValueSnapshot,
    SettingWidgetSnapshot,
} from '../runtimeProtocol';
import { GuestMemory } from './memory.ts';

type SettingValue =
    | { type: 'map'; value: Map<string, SettingValue> }
    | { type: 'list'; value: SettingValue[] }
    | { type: 'bool'; value: boolean }
    | { type: 'i64'; value: bigint }
    | { type: 'f64'; value: number }
    | { type: 'string'; value: string };

interface MapHandle {
    value: Map<string, SettingValue>;
    rootRevision?: number;
}

class Handles<T> {
    private readonly values = new Map<bigint, T>();
    private next = 1n;

    public allocate(value: T): bigint {
        const handle = this.next++;
        this.values.set(handle, value);
        return handle;
    }

    public get(handle: bigint, kind: string): T {
        const value = this.values.get(handle);
        if (value === undefined) {
            throw new WebAssembly.RuntimeError(`Invalid ${kind} handle: ${handle}`);
        }
        return value;
    }

    public free(handle: bigint, kind: string): void {
        if (!this.values.delete(handle)) {
            throw new WebAssembly.RuntimeError(`Invalid ${kind} handle: ${handle}`);
        }
    }

    public get size(): number {
        return this.values.size;
    }
}

export class SettingsHost {
    private readonly memory: GuestMemory;
    private readonly maps = new Handles<MapHandle>();
    private readonly lists = new Handles<SettingValue[]>();
    private readonly values = new Handles<SettingValue>();
    private readonly widgets: SettingWidgetSnapshot[] = [];
    private root: Map<string, SettingValue>;
    private rootRevision = 0;
    private changed = true;

    public constructor(
        memory: GuestMemory,
        initial: SettingMapSnapshot = [],
    ) {
        this.memory = memory;
        this.root = mapFromSnapshot(initial);
    }

    public snapshot(): SettingsSnapshot {
        return {
            widgets: structuredClone(this.widgets),
            map: mapSnapshot(this.root),
            handleCount: this.maps.size + this.lists.size + this.values.size,
        };
    }

    public consumeChanged(): boolean {
        const changed = this.changed;
        this.changed = false;
        return changed;
    }

    public set(key: string, value: boolean | string): void {
        this.root.set(key, typeof value === 'boolean'
            ? { type: 'bool', value }
            : { type: 'string', value });
        this.rootRevision += 1;
        this.changed = true;
    }

    public clear(): void {
        this.root = new Map();
        this.rootRevision += 1;
        this.changed = true;
    }

    public imports(): Record<string, WebAssembly.ImportValue> {
        return {
            settings_map_new: () => this.maps.allocate({ value: new Map() }),
            settings_map_free: (handle: bigint) => this.maps.free(handle, 'settings map'),
            settings_map_load: () => this.maps.allocate({
                value: cloneMap(this.root),
                rootRevision: this.rootRevision,
            }),
            settings_map_store: (handle: bigint) => this.storeMap(this.maps.get(handle, 'settings map')),
            settings_map_store_if_unchanged: (oldHandle: bigint, newHandle: bigint) => {
                const old = this.maps.get(oldHandle, 'settings map');
                const next = this.maps.get(newHandle, 'settings map');
                if (old.rootRevision !== this.rootRevision) {
                    return 0;
                }
                this.storeMap(next);
                return 1;
            },
            settings_map_copy: (handle: bigint) => {
                const map = this.maps.get(handle, 'settings map');
                return this.maps.allocate({ value: cloneMap(map.value), rootRevision: map.rootRevision });
            },
            settings_map_insert: (handle: bigint, keyPointer: number, keyLength: number, value: bigint) => {
                this.maps.get(handle, 'settings map').value.set(
                    this.memory.readString(keyPointer, keyLength),
                    cloneValue(this.values.get(value, 'setting value')),
                );
            },
            settings_map_get: (handle: bigint, keyPointer: number, keyLength: number) => {
                const value = this.maps.get(handle, 'settings map').value.get(
                    this.memory.readString(keyPointer, keyLength),
                );
                return value === undefined ? 0n : this.values.allocate(cloneValue(value));
            },
            settings_map_len: (handle: bigint) => BigInt(this.maps.get(handle, 'settings map').value.size),
            settings_map_get_key_by_index: (
                handle: bigint,
                index: bigint,
                pointer: number,
                lengthPointer: number,
            ) => {
                const entry = entryAt(this.maps.get(handle, 'settings map').value, index);
                return entry === undefined
                    ? this.writeMissingString(lengthPointer)
                    : this.memory.writeHostString(pointer, lengthPointer, entry[0]);
            },
            settings_map_get_value_by_index: (handle: bigint, index: bigint) => {
                const entry = entryAt(this.maps.get(handle, 'settings map').value, index);
                return entry === undefined ? 0n : this.values.allocate(cloneValue(entry[1]));
            },
            settings_list_new: () => this.lists.allocate([]),
            settings_list_free: (handle: bigint) => this.lists.free(handle, 'settings list'),
            settings_list_copy: (handle: bigint) => this.lists.allocate(
                this.lists.get(handle, 'settings list').map(cloneValue),
            ),
            settings_list_len: (handle: bigint) => BigInt(this.lists.get(handle, 'settings list').length),
            settings_list_get: (handle: bigint, index: bigint) => {
                const value = arrayAt(this.lists.get(handle, 'settings list'), index);
                return value === undefined ? 0n : this.values.allocate(cloneValue(value));
            },
            settings_list_set: (handle: bigint, index: bigint, value: bigint) => {
                const list = this.lists.get(handle, 'settings list');
                const position = safeIndex(index);
                if (position === undefined || position >= list.length) {
                    return 0;
                }
                list[position] = cloneValue(this.values.get(value, 'setting value'));
                return 1;
            },
            settings_list_push: (handle: bigint, value: bigint) => {
                this.lists.get(handle, 'settings list').push(
                    cloneValue(this.values.get(value, 'setting value')),
                );
            },
            settings_list_insert: (handle: bigint, index: bigint, value: bigint) => {
                const list = this.lists.get(handle, 'settings list');
                const position = safeIndex(index);
                if (position === undefined || position > list.length) {
                    return 0;
                }
                list.splice(position, 0, cloneValue(this.values.get(value, 'setting value')));
                return 1;
            },
            settings_list_remove: (handle: bigint, index: bigint) => {
                const list = this.lists.get(handle, 'settings list');
                const position = safeIndex(index);
                if (position === undefined || position >= list.length) {
                    return 0n;
                }
                return this.values.allocate(list.splice(position, 1)[0]);
            },
            setting_value_new_map: (handle: bigint) => this.values.allocate({
                type: 'map',
                value: cloneMap(this.maps.get(handle, 'settings map').value),
            }),
            setting_value_new_list: (handle: bigint) => this.values.allocate({
                type: 'list',
                value: this.lists.get(handle, 'settings list').map(cloneValue),
            }),
            setting_value_new_bool: (value: number) => this.values.allocate({ type: 'bool', value: value !== 0 }),
            setting_value_new_i64: (value: bigint) => this.values.allocate({ type: 'i64', value }),
            setting_value_new_f64: (value: number) => this.values.allocate({ type: 'f64', value }),
            setting_value_new_string: (pointer: number, length: number) => this.values.allocate({
                type: 'string',
                value: this.memory.readString(pointer, length),
            }),
            setting_value_free: (handle: bigint) => this.values.free(handle, 'setting value'),
            setting_value_copy: (handle: bigint) => this.values.allocate(
                cloneValue(this.values.get(handle, 'setting value')),
            ),
            setting_value_get_type: (handle: bigint) => valueType(this.values.get(handle, 'setting value')),
            setting_value_get_map: (handle: bigint, pointer: number) => {
                const value = this.values.get(handle, 'setting value');
                if (value.type !== 'map') return 0;
                this.memory.writeU64(pointer, this.maps.allocate({ value: cloneMap(value.value) }));
                return 1;
            },
            setting_value_get_list: (handle: bigint, pointer: number) => {
                const value = this.values.get(handle, 'setting value');
                if (value.type !== 'list') return 0;
                this.memory.writeU64(pointer, this.lists.allocate(value.value.map(cloneValue)));
                return 1;
            },
            setting_value_get_bool: (handle: bigint, pointer: number) => {
                const value = this.values.get(handle, 'setting value');
                if (value.type !== 'bool') return 0;
                this.memory.writeU8(pointer, value.value ? 1 : 0);
                return 1;
            },
            setting_value_get_i64: (handle: bigint, pointer: number) => {
                const value = this.values.get(handle, 'setting value');
                if (value.type !== 'i64') return 0;
                this.memory.writeI64(pointer, value.value);
                return 1;
            },
            setting_value_get_f64: (handle: bigint, pointer: number) => {
                const value = this.values.get(handle, 'setting value');
                if (value.type !== 'f64') return 0;
                this.memory.writeF64(pointer, value.value);
                return 1;
            },
            setting_value_get_string: (handle: bigint, pointer: number, lengthPointer: number) => {
                const value = this.values.get(handle, 'setting value');
                if (value.type !== 'string') return this.writeMissingString(lengthPointer);
                return this.memory.writeHostString(pointer, lengthPointer, value.value);
            },
            user_settings_add_bool: (
                keyPointer: number,
                keyLength: number,
                descriptionPointer: number,
                descriptionLength: number,
                defaultValue: number,
            ) => this.addBool(keyPointer, keyLength, descriptionPointer, descriptionLength, defaultValue),
            user_settings_add_title: (
                keyPointer: number,
                keyLength: number,
                descriptionPointer: number,
                descriptionLength: number,
                headingLevel: number,
            ) => this.addWidget({
                type: 'title',
                key: this.memory.readString(keyPointer, keyLength),
                description: this.memory.readString(descriptionPointer, descriptionLength),
                headingLevel,
            }),
            user_settings_add_choice: (
                keyPointer: number,
                keyLength: number,
                descriptionPointer: number,
                descriptionLength: number,
                defaultPointer: number,
                defaultLength: number,
            ) => this.addWidget({
                type: 'choice',
                key: this.memory.readString(keyPointer, keyLength),
                description: this.memory.readString(descriptionPointer, descriptionLength),
                defaultOptionKey: this.memory.readString(defaultPointer, defaultLength),
                options: [],
            }),
            user_settings_add_choice_option: (
                keyPointer: number,
                keyLength: number,
                optionPointer: number,
                optionLength: number,
                descriptionPointer: number,
                descriptionLength: number,
            ) => this.addChoiceOption(
                this.memory.readString(keyPointer, keyLength),
                this.memory.readString(optionPointer, optionLength),
                this.memory.readString(descriptionPointer, descriptionLength),
            ),
            user_settings_add_file_select: (
                keyPointer: number,
                keyLength: number,
                descriptionPointer: number,
                descriptionLength: number,
            ) => this.addWidget({
                type: 'fileSelect',
                key: this.memory.readString(keyPointer, keyLength),
                description: this.memory.readString(descriptionPointer, descriptionLength),
                filters: [],
            }),
            user_settings_add_file_select_name_filter: (
                keyPointer: number,
                keyLength: number,
                descriptionPointer: number,
                descriptionLength: number,
                patternPointer: number,
                patternLength: number,
            ) => this.addFileFilter(
                this.memory.readString(keyPointer, keyLength),
                {
                    type: 'name',
                    description: descriptionPointer === 0
                        ? undefined
                        : this.memory.readString(descriptionPointer, descriptionLength),
                    pattern: this.memory.readString(patternPointer, patternLength),
                },
            ),
            user_settings_add_file_select_mime_filter: (
                keyPointer: number,
                keyLength: number,
                mimePointer: number,
                mimeLength: number,
            ) => this.addFileFilter(
                this.memory.readString(keyPointer, keyLength),
                { type: 'mime', mime: this.memory.readString(mimePointer, mimeLength) },
            ),
            user_settings_add_text_input: (
                keyPointer: number,
                keyLength: number,
                descriptionPointer: number,
                descriptionLength: number,
                defaultPointer: number,
                defaultLength: number,
            ) => this.addWidget({
                type: 'textInput',
                key: this.memory.readString(keyPointer, keyLength),
                description: this.memory.readString(descriptionPointer, descriptionLength),
                defaultValue: this.memory.readString(defaultPointer, defaultLength),
            }),
            user_settings_set_tooltip: (
                keyPointer: number,
                keyLength: number,
                tooltipPointer: number,
                tooltipLength: number,
            ) => this.setTooltip(
                this.memory.readString(keyPointer, keyLength),
                this.memory.readString(tooltipPointer, tooltipLength),
            ),
        };
    }

    private storeMap(map: MapHandle): void {
        this.root = cloneMap(map.value);
        this.rootRevision += 1;
        this.changed = true;
    }

    private writeMissingString(lengthPointer: number): number {
        this.memory.writeU32(lengthPointer, 0);
        return 0;
    }

    private addBool(
        keyPointer: number,
        keyLength: number,
        descriptionPointer: number,
        descriptionLength: number,
        defaultValue: number,
    ): number {
        const key = this.memory.readString(keyPointer, keyLength);
        const fallback = defaultValue !== 0;
        this.addWidget({
            type: 'bool',
            key,
            description: this.memory.readString(descriptionPointer, descriptionLength),
            defaultValue: fallback,
        });
        const stored = this.root.get(key);
        return stored?.type === 'bool' ? Number(stored.value) : Number(fallback);
    }

    private addWidget(widget: SettingWidgetSnapshot): void {
        if (this.widgets.some(existing => existing.key === widget.key)) {
            throw new WebAssembly.RuntimeError(`Duplicate user setting key: ${widget.key}`);
        }
        this.widgets.push(widget);
        this.changed = true;
    }

    private addChoiceOption(key: string, optionKey: string, description: string): number {
        const widget = this.widget(key);
        if (widget.type !== 'choice') {
            throw new WebAssembly.RuntimeError('The setting is not a choice.');
        }
        widget.options.push({ key: optionKey, description });
        this.changed = true;
        const stored = this.root.get(key);
        const selected = stored?.type === 'string' ? stored.value : widget.defaultOptionKey;
        return Number(selected === optionKey);
    }

    private addFileFilter(
        key: string,
        filter: Extract<SettingWidgetSnapshot, { type: 'fileSelect' }>['filters'][number],
    ): void {
        const widget = this.widget(key);
        if (widget.type !== 'fileSelect') {
            throw new WebAssembly.RuntimeError('The setting is not a file select.');
        }
        widget.filters.push(filter);
        this.changed = true;
    }

    private setTooltip(key: string, tooltip: string): void {
        this.widget(key).tooltip = tooltip;
        this.changed = true;
    }

    private widget(key: string): SettingWidgetSnapshot {
        const widget = this.widgets.find(candidate => candidate.key === key);
        if (widget === undefined) {
            throw new WebAssembly.RuntimeError('There is no setting with the provided key.');
        }
        return widget;
    }
}

function valueType(value: SettingValue): number {
    return { map: 1, list: 2, bool: 3, i64: 4, f64: 5, string: 6 }[value.type];
}

function safeIndex(value: bigint): number | undefined {
    return value >= 0n && value <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(value) : undefined;
}

function arrayAt<T>(values: T[], index: bigint): T | undefined {
    const position = safeIndex(index);
    return position === undefined ? undefined : values[position];
}

function entryAt<K, V>(values: Map<K, V>, index: bigint): [K, V] | undefined {
    const position = safeIndex(index);
    if (position === undefined || position >= values.size) return undefined;
    return [...values.entries()][position];
}

function cloneMap(value: Map<string, SettingValue>): Map<string, SettingValue> {
    return new Map([...value].map(([key, item]) => [key, cloneValue(item)]));
}

function cloneValue(value: SettingValue): SettingValue {
    if (value.type === 'map') return { type: 'map', value: cloneMap(value.value) };
    if (value.type === 'list') return { type: 'list', value: value.value.map(cloneValue) };
    return { ...value };
}

function mapSnapshot(value: Map<string, SettingValue>): SettingMapSnapshot {
    return [...value].map(([key, item]) => ({ key, value: valueSnapshot(item) }));
}

function valueSnapshot(value: SettingValue): SettingValueSnapshot {
    if (value.type === 'map') return { type: 'map', value: mapSnapshot(value.value) };
    if (value.type === 'list') return { type: 'list', value: value.value.map(valueSnapshot) };
    if (value.type === 'i64') return { type: 'i64', value: value.value.toString() };
    return { ...value };
}

function mapFromSnapshot(value: SettingMapSnapshot): Map<string, SettingValue> {
    return new Map(value.map(entry => [entry.key, valueFromSnapshot(entry.value)]));
}

function valueFromSnapshot(value: SettingValueSnapshot): SettingValue {
    if (value.type === 'map') return { type: 'map', value: mapFromSnapshot(value.value) };
    if (value.type === 'list') return { type: 'list', value: value.value.map(valueFromSnapshot) };
    if (value.type === 'i64') return { type: 'i64', value: BigInt(value.value) };
    return { ...value };
}
