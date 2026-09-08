import type {
    GameTimeState,
    RuntimeLogMessage,
    TimerSnapshot,
    TimerState,
} from '../runtimeProtocol';

const TIMER_STATE_NUMBER: Readonly<Record<TimerState, number>> = {
    notRunning: 0,
    running: 1,
    paused: 2,
    ended: 3,
};

export type LogSink = (message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>) => void;

export class DebuggerTimer {
    private readonly onChanged: () => void;
    private readonly log: LogSink;
    private state: TimerState = 'notRunning';
    private gameTimeSeconds = 0;
    private gameTimeState: GameTimeState = 'notInitialized';
    private splitIndex = 0;
    private readonly segmentsSplit: boolean[] = [];
    private readonly variables = new Map<string, string>();

    public constructor(
        onChanged: () => void,
        log: LogSink,
    ) {
        this.onChanged = onChanged;
        this.log = log;
    }

    public stateNumber(): number {
        return TIMER_STATE_NUMBER[this.state];
    }

    public currentSplitIndex(): bigint {
        return this.state === 'notRunning' ? -1n : BigInt(this.splitIndex);
    }

    public segmentSplitted(index: bigint): number {
        if (index < 0n || index > BigInt(Number.MAX_SAFE_INTEGER)) {
            return -1;
        }
        const number = Number(index);
        if (this.state === 'notRunning' || number >= this.splitIndex) {
            return -1;
        }
        const value = this.segmentsSplit[number];
        return value === undefined ? -1 : Number(value);
    }

    public start(): void {
        if (this.state !== 'notRunning') {
            return;
        }
        this.state = 'running';
        this.runtimeLog('Timer started.');
        this.onChanged();
    }

    public split(): void {
        if (this.state !== 'running') {
            return;
        }
        this.splitIndex += 1;
        this.segmentsSplit.push(true);
        this.runtimeLog('Splitted.');
        this.onChanged();
    }

    public skipSplit(): void {
        if (this.state !== 'running') {
            return;
        }
        this.splitIndex += 1;
        this.segmentsSplit.push(false);
        this.runtimeLog('Split skipped.');
        this.onChanged();
    }

    public undoSplit(): void {
        if (this.state === 'ended') {
            this.state = 'running';
        }
        if (this.state !== 'running') {
            return;
        }
        this.splitIndex = Math.max(0, this.splitIndex - 1);
        this.segmentsSplit.length = this.splitIndex;
        this.runtimeLog('Split undone.');
        this.onChanged();
    }

    public reset(): void {
        this.state = 'notRunning';
        this.gameTimeSeconds = 0;
        this.gameTimeState = 'notInitialized';
        this.splitIndex = 0;
        this.segmentsSplit.length = 0;
        this.variables.clear();
        this.runtimeLog('Run reset.');
        this.onChanged();
    }

    public setGameTime(seconds: bigint, nanoseconds: number): void {
        this.gameTimeSeconds = Number(seconds) + nanoseconds / 1_000_000_000;
        if (this.gameTimeState === 'notInitialized') {
            this.gameTimeState = 'running';
        }
        this.onChanged();
    }

    public pauseGameTime(): void {
        this.gameTimeState = 'paused';
        this.onChanged();
    }

    public resumeGameTime(): void {
        this.gameTimeState = 'running';
        this.onChanged();
    }

    public setVariable(name: string, value: string): void {
        this.variables.set(name, value);
        this.onChanged();
    }

    public snapshot(): TimerSnapshot {
        return {
            state: this.state,
            gameTimeSeconds: this.gameTimeSeconds,
            gameTimeState: this.gameTimeState,
            splitIndex: this.splitIndex,
            variables: Object.fromEntries(this.variables),
        };
    }

    private runtimeLog(message: string): void {
        this.log({
            source: 'runtime',
            level: 'debug',
            message,
        });
    }
}
