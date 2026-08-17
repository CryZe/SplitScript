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
        };
    };
}

const manifest = JSON.parse(
    readFileSync(fileURLToPath(new URL('../package.json', import.meta.url)), 'utf8'),
) as ExtensionManifest;

test('documentation has direct and searchable commands', () => {
    const commands = new Set(manifest.contributes.commands.map(command => command.command));
    assert(commands.has('splitscript.openDocumentation'));
    assert(commands.has('splitscript.searchDocumentation'));
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
