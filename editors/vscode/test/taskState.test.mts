import assert from 'node:assert/strict';
import test from 'node:test';
import { ExclusiveTaskState } from '../src/taskState.ts';

interface Task {
    kind: 'release' | 'watch';
}

test('only one compiler task can own the controller', () => {
    const state = new ExclusiveTaskState<Task>();
    const release: Task = { kind: 'release' };
    const watch: Task = { kind: 'watch' };
    assert.equal(state.begin(release), true);
    assert.equal(state.begin(watch), false);
    assert.equal(state.current, release);
});

test('a stale close event cannot clear a newer task', () => {
    const state = new ExclusiveTaskState<Task>();
    const firstWatch: Task = { kind: 'watch' };
    const release: Task = { kind: 'release' };
    assert.equal(state.begin(firstWatch), true);
    assert.equal(state.finish(firstWatch), true);
    assert.equal(state.begin(release), true);
    assert.equal(state.finish(firstWatch), false);
    assert.equal(state.current, release);
});

test('completion releases ownership exactly once', () => {
    const state = new ExclusiveTaskState<Task>();
    const release: Task = { kind: 'release' };
    assert.equal(state.begin(release), true);
    assert.equal(state.finish(release), true);
    assert.equal(state.finish(release), false);
    assert.equal(state.current, undefined);
});
