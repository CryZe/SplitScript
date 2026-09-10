export interface TickStatisticsSnapshot {
    sampleCount: number;
    retainedSampleCount: number;
    averageMilliseconds: number;
    slowestMilliseconds: number;
}

export class TickStatistics {
    private readonly samples: Float64Array;
    private cursor = 0;
    private retainedSampleCount = 0;
    private sampleCount = 0;
    private sumMilliseconds = 0;
    private slowestMilliseconds = 0;

    public constructor(capacity: number) {
        if (!Number.isInteger(capacity) || capacity <= 0) {
            throw new RangeError('tick statistics capacity must be a positive integer');
        }
        this.samples = new Float64Array(capacity);
    }

    public record(milliseconds: number): void {
        if (!Number.isFinite(milliseconds) || milliseconds < 0) {
            throw new RangeError('tick duration must be finite and non-negative');
        }
        if (this.retainedSampleCount === this.samples.length) {
            this.sumMilliseconds -= this.samples[this.cursor];
        } else {
            this.retainedSampleCount += 1;
        }
        this.samples[this.cursor] = milliseconds;
        this.cursor = (this.cursor + 1) % this.samples.length;
        this.sampleCount += 1;
        this.sumMilliseconds += milliseconds;
        this.slowestMilliseconds = Math.max(this.slowestMilliseconds, milliseconds);
    }

    public snapshot(): TickStatisticsSnapshot {
        return {
            sampleCount: this.sampleCount,
            retainedSampleCount: this.retainedSampleCount,
            averageMilliseconds: this.retainedSampleCount === 0
                ? 0
                : this.sumMilliseconds / this.retainedSampleCount,
            slowestMilliseconds: this.slowestMilliseconds,
        };
    }

    public recentSamples(): number[] {
        const start = (this.cursor - this.retainedSampleCount + this.samples.length)
            % this.samples.length;
        return Array.from(
            { length: this.retainedSampleCount },
            (_, index) => this.samples[(start + index) % this.samples.length],
        );
    }

    public reset(): void {
        this.cursor = 0;
        this.retainedSampleCount = 0;
        this.sampleCount = 0;
        this.sumMilliseconds = 0;
        this.slowestMilliseconds = 0;
    }
}
