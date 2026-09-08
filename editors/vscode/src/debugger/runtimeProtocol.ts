export type TimerState = 'notRunning' | 'running' | 'paused' | 'ended';
export type GameTimeState = 'notInitialized' | 'paused' | 'running';

export interface TimerSnapshot {
    state: TimerState;
    gameTimeSeconds: number;
    gameTimeState: GameTimeState;
    splitIndex: number;
    variables: Readonly<Record<string, string>>;
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
}

export interface RuntimeLaunchMessage {
    type: 'launch';
    wasm: ArrayBuffer;
    program: string;
}

export interface RuntimeTimerCommandMessage {
    type: 'timerCommand';
    command: 'start' | 'reset';
}

export type RuntimeRequest = RuntimeLaunchMessage | RuntimeTimerCommandMessage;

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
