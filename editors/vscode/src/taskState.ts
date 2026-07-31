/** Identity-safe exclusive ownership for release builds and watch sessions. */
export class ExclusiveTaskState<T extends object> {
    private active: T | undefined;

    public get current(): T | undefined {
        return this.active;
    }

    public begin(task: T): boolean {
        if (this.active !== undefined) {
            return false;
        }
        this.active = task;
        return true;
    }

    public finish(task: T): boolean {
        if (this.active !== task) {
            return false;
        }
        this.active = undefined;
        return true;
    }
}
