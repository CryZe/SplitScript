import assert from 'node:assert/strict';
import test from 'node:test';
import {
    resolveSavedScript,
    type ScriptDocument,
    type ScriptResource,
    type ScriptSaveHost,
} from '../src/savedScript.ts';

class Resource implements ScriptResource {
    public readonly scheme: string;
    private readonly value: string;

    public constructor(
        scheme: string,
        value: string,
    ) {
        this.scheme = scheme;
        this.value = value;
    }

    public toString(): string {
        return this.value;
    }
}

interface Document extends ScriptDocument<Resource> {
    uri: Resource;
    languageId: string;
    isDirty: boolean;
    isUntitled: boolean;
    isClosed: boolean;
}

function document(uri: Resource, overrides: Partial<Document> = {}): Document {
    return {
        uri,
        languageId: 'splitscript',
        isDirty: false,
        isUntitled: false,
        isClosed: false,
        ...overrides,
    };
}

function host(
    active: () => Document | undefined,
    overrides: Partial<ScriptSaveHost<Resource, Document>> = {},
): ScriptSaveHost<Resource, Document> {
    return {
        activeDocument: active,
        save: async resource => resource,
        saveAs: async resource => resource,
        openDocument: async () => {
            throw new Error('the test did not expect the document to be reopened');
        },
        ...overrides,
    };
}

test('a focus change during save cannot replace the selected script', async () => {
    const selectedUri = new Resource('file', 'file:///selected.split');
    const otherUri = new Resource('file', 'file:///other.split');
    const selected = document(selectedUri, { isDirty: true });
    const other = document(otherUri);
    let active = selected;
    let activeReads = 0;

    const result = await resolveSavedScript(host(
        () => {
            activeReads += 1;
            return active;
        },
        {
            save: async resource => {
                assert.equal(resource, selectedUri);
                selected.isDirty = false;
                active = other;
                return selectedUri;
            },
        },
    ));

    assert.equal(activeReads, 1);
    assert.ok('document' in result);
    assert.equal(result.document, selected);
});

test('untitled Save As follows the returned resource instead of editor focus', async () => {
    const untitledUri = new Resource('untitled', 'untitled:Untitled-1');
    const savedUri = new Resource('file', 'file:///saved.split');
    const otherUri = new Resource('file', 'file:///other.split');
    const untitled = document(untitledUri, { isDirty: true, isUntitled: true });
    const saved = document(savedUri);
    const other = document(otherUri);
    let active = untitled;

    const result = await resolveSavedScript(host(
        () => active,
        {
            saveAs: async resource => {
                assert.equal(resource, untitledUri);
                untitled.isClosed = true;
                active = other;
                return savedUri;
            },
            openDocument: async resource => {
                assert.equal(resource, savedUri);
                return saved;
            },
        },
    ));

    assert.ok('document' in result);
    assert.equal(result.document, saved);
});

test('cancelled or failed saves do not select another document', async () => {
    const selectedUri = new Resource('file', 'file:///selected.split');
    const selected = document(selectedUri, { isDirty: true });
    const result = await resolveSavedScript(host(
        () => selected,
        { save: async () => undefined },
    ));
    assert.deepEqual(result, { failure: 'saveFailed' });
});

test('the returned Save As document is revalidated', async () => {
    const untitledUri = new Resource('untitled', 'untitled:Untitled-1');
    const savedUri = new Resource('file', 'file:///saved.txt');
    const untitled = document(untitledUri, { isUntitled: true });
    const plainText = document(savedUri, { languageId: 'plaintext' });
    const result = await resolveSavedScript(host(
        () => untitled,
        {
            saveAs: async () => savedUri,
            openDocument: async () => plainText,
        },
    ));
    assert.deepEqual(result, { failure: 'wrongLanguage' });
});
