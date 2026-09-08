export type TimerState = 'notRunning' | 'running' | 'paused' | 'ended';
export type GameTimeState = 'notInitialized' | 'paused' | 'running';

export interface TimerSnapshot {
    state: TimerState;
    gameTimeSeconds: number;
    gameTimeState: GameTimeState;
    splitIndex: number;
    variables: Readonly<Record<string, string>>;
}

export type SettingValueSnapshot =
    | { type: 'map'; value: SettingMapSnapshot }
    | { type: 'list'; value: SettingValueSnapshot[] }
    | { type: 'bool'; value: boolean }
    | { type: 'i64'; value: string }
    | { type: 'f64'; value: number }
    | { type: 'string'; value: string };

export type SettingMapSnapshot = Array<{
    key: string;
    value: SettingValueSnapshot;
}>;

export interface SettingWidgetBase {
    key: string;
    description: string;
    tooltip?: string;
}

export type SettingWidgetSnapshot =
    | (SettingWidgetBase & { type: 'title'; headingLevel: number })
    | (SettingWidgetBase & { type: 'bool'; defaultValue: boolean })
    | (SettingWidgetBase & {
        type: 'choice';
        defaultOptionKey: string;
        options: Array<{ key: string; description: string }>;
    })
    | (SettingWidgetBase & {
        type: 'fileSelect';
        filters: Array<
            | { type: 'name'; description?: string; pattern: string }
            | { type: 'mime'; mime: string }
        >;
    })
    | (SettingWidgetBase & { type: 'textInput'; defaultValue: string });

export interface SettingsSnapshot {
    widgets: SettingWidgetSnapshot[];
    map: SettingMapSnapshot;
    handleCount: number;
}

export interface RuntimeSnapshot {
    status: 'starting' | 'running' | 'trapped';
    program: string;
    tickRateHz: number;
    tickCount: number;
    averageTickMilliseconds: number;
    slowestTickMilliseconds: number;
    memoryBytes: number;
    timer: TimerSnapshot;
    settings: SettingsSnapshot;
}

export interface RuntimeLaunchMessage {
    type: 'launch';
    wasm: ArrayBuffer;
    program: string;
    scriptPath?: string;
    settings?: SettingMapSnapshot;
}

export interface RuntimeTimerCommandMessage {
    type: 'timerCommand';
    command: 'start' | 'reset';
}

export interface RuntimeSetSettingMessage {
    type: 'setSetting';
    key: string;
    value: boolean | string;
}

export interface RuntimeClearSettingsMessage {
    type: 'clearSettings';
}

export type RuntimeRequest =
    | RuntimeLaunchMessage
    | RuntimeTimerCommandMessage
    | RuntimeSetSettingMessage
    | RuntimeClearSettingsMessage;

export interface RuntimeReadyMessage {
    type: 'ready';
    unsupportedImports: string[];
}

export interface RuntimeSnapshotMessage {
    type: 'snapshot';
    snapshot: RuntimeSnapshot;
}

export interface RuntimeLogMessage {
    type: 'log';
    timestamp: string;
    source: 'runtime' | 'autoSplitter';
    level: 'debug' | 'info' | 'warning' | 'error';
    message: string;
}

export interface RuntimeFailureMessage {
    type: 'failure';
    message: string;
    stack?: string;
}

export type RuntimeResponse =
    | RuntimeReadyMessage
    | RuntimeSnapshotMessage
    | RuntimeLogMessage
    | RuntimeFailureMessage;
