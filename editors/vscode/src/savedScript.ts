export interface ScriptResource {
    readonly scheme: string;
    toString(): string;
}

export interface ScriptDocument<Resource extends ScriptResource> {
    readonly uri: Resource;
    readonly languageId: string;
    readonly isDirty: boolean;
    readonly isUntitled: boolean;
    readonly isClosed: boolean;
}

export interface ScriptSaveHost<
    Resource extends ScriptResource,
    Document extends ScriptDocument<Resource>,
> {
    activeDocument(): Document | undefined;
    save(resource: Resource): PromiseLike<Resource | undefined>;
    saveAs(resource: Resource): PromiseLike<Resource | undefined>;
    openDocument(resource: Resource): PromiseLike<Document>;
}

export type SavedScriptFailure =
    | 'noScript'
    | 'saveFailed'
    | 'notSaved'
    | 'wrongLanguage'
    | 'wrongResource';

export type SavedScriptResult<Document> =
    | { readonly document: Document }
    | { readonly failure: SavedScriptFailure };

/**
 * Resolves the exact SplitScript document selected when a command starts.
 *
 * Saving may yield to a formatter, a Save As dialog, and arbitrary editor
 * focus changes. The workspace save APIs return the resulting resource, so a
 * command can follow that identity without ever consulting the active editor
 * a second time.
 */
export async function resolveSavedScript<
    Resource extends ScriptResource,
    Document extends ScriptDocument<Resource>,
>(host: ScriptSaveHost<Resource, Document>): Promise<SavedScriptResult<Document>> {
    const selected = host.activeDocument();
    if (selected === undefined || selected.languageId !== 'splitscript') {
        return { failure: 'noScript' };
    }

    let savedResource = selected.uri;
    if (selected.isUntitled) {
        const result = await host.saveAs(selected.uri);
        if (result === undefined) {
            return { failure: 'saveFailed' };
        }
        savedResource = result;
    } else if (selected.isDirty) {
        const result = await host.save(selected.uri);
        if (result === undefined) {
            return { failure: 'saveFailed' };
        }
        savedResource = result;
    }

    if (savedResource.scheme === 'untitled') {
        return { failure: 'notSaved' };
    }

    const savedKey = savedResource.toString();
    const document = !selected.isClosed && selected.uri.toString() === savedKey
        ? selected
        : await host.openDocument(savedResource);
    if (document.isClosed || document.uri.toString() !== savedKey) {
        return { failure: 'wrongResource' };
    }
    if (document.isUntitled) {
        return { failure: 'notSaved' };
    }
    if (document.languageId !== 'splitscript') {
        return { failure: 'wrongLanguage' };
    }
    return { document };
}
