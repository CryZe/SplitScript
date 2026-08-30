import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { documentationMarkdownTrust } from '../src/documentationMarkdown.ts';

interface CommandContribution {
    command: string;
}

interface MenuContribution {
    command: string;
    when?: string;
    group?: string;
}

interface ExtensionManifest {
    contributes: {
        commands: CommandContribution[];
        menus: {
            'editor/title': MenuContribution[];
            'editor/context': MenuContribution[];
        };
        configurationDefaults: {
            '[splitscript]': Record<string, unknown>;
        };
    };
}

const manifest = JSON.parse(
    readFileSync(fileURLToPath(new URL('../package.json', import.meta.url)), 'utf8'),
) as ExtensionManifest;
const documentationStyles = readFileSync(
    fileURLToPath(new URL('../styles/documentation.css', import.meta.url)),
    'utf8',
);

test('documentation has direct, contextual, and searchable commands', () => {
    const commands = new Set(manifest.contributes.commands.map(command => command.command));
    assert(commands.has('splitscript.openDocumentation'));
    assert(commands.has('splitscript.openSymbolDocumentation'));
    assert(commands.has('splitscript.searchDocumentation'));
});

test('symbol documentation is available from the SplitScript editor context', () => {
    const contribution = manifest.contributes.menus['editor/context'].find(
        item => item.command === 'splitscript.openSymbolDocumentation',
    );
    assert.deepEqual(contribution, {
        command: 'splitscript.openSymbolDocumentation',
        when: 'editorLangId == splitscript',
        group: 'navigation@3',
    });
});

test('the direct documentation command is available in SplitScript editor titles', () => {
    const contribution = manifest.contributes.menus['editor/title'].find(
        item => item.command === 'splitscript.openDocumentation',
    );
    assert.deepEqual(contribution, {
        command: 'splitscript.openDocumentation',
        when: 'resourceLangId == splitscript',
        group: 'navigation@3',
    });
});

test('language-server documentation links trust only the documentation command', () => {
    assert.deepEqual(documentationMarkdownTrust, {
        isTrusted: {
            enabledCommands: ['splitscript.openDocumentation'],
        },
    });
});

test('SplitScript inherits the user formatting policy', () => {
    const defaults = manifest.contributes.configurationDefaults['[splitscript]'];
    assert(!Object.hasOwn(defaults, 'editor.formatOnSave'));
    assert(!Object.hasOwn(defaults, 'editor.defaultFormatter'));
});

test('documentation gives enum variants their dedicated palette color', () => {
    assert.match(
        documentationStyles,
        /\[data-splitscript-token="enumMember"\]\s*\{\s*color:\s*#F397FF;/,
    );
});
