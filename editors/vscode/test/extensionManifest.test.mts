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
    categories: string[];
    contributes: {
        commands: CommandContribution[];
        debuggers: Array<{
            type: string;
            languages: string[];
            configurationAttributes: {
                launch: {
                    required: string[];
                    properties: Record<string, unknown>;
                };
            };
        }>;
        viewsContainers: {
            activitybar: Array<{ id: string; title: string; icon: string }>;
        };
        views: {
            'splitscript-debugger': Array<{ id: string; name: string }>;
        };
        viewsWelcome: Array<{ view: string; when?: string }>;
        menus: {
            'editor/title': MenuContribution[];
            'editor/context': MenuContribution[];
            'view/title': MenuContribution[];
        };
        configurationDefaults: {
            '[splitscript]': Record<string, unknown>;
        };
        configuration: {
            properties: Record<string, { default: unknown }>;
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

test('desktop debugger contribution launches SplitScript files', () => {
    assert(manifest.categories.includes('Debuggers'));
    const debuggerContribution = manifest.contributes.debuggers.find(
        contribution => contribution.type === 'splitscript',
    );
    assert(debuggerContribution !== undefined);
    assert.deepEqual(debuggerContribution.languages, ['splitscript']);
    assert.deepEqual(debuggerContribution.configurationAttributes.launch.required, ['program']);
    assert('hotReload' in debuggerContribution.configurationAttributes.launch.properties);
    assert('scriptPath' in debuggerContribution.configurationAttributes.launch.properties);
});

test('runtime view has launch welcome content and active-session actions', () => {
    const [container] = manifest.contributes.viewsContainers.activitybar;
    assert.match(container.id, /^[A-Za-z0-9_-]+$/);
    assert.deepEqual(manifest.contributes.viewsContainers.activitybar, [
        {
            id: 'splitscript-debugger',
            title: 'SplitScript Debugger',
            icon: 'media/splitscript-debugger.svg',
        },
    ]);
    assert.deepEqual(
        manifest.contributes.views['splitscript-debugger'].map(view => view.id),
        [
            'splitscript.debugger.runtime',
            'splitscript.debugger.statistics',
            'splitscript.debugger.settings',
            'splitscript.debugger.settingsMap',
            'splitscript.debugger.variables',
            'splitscript.debugger.processes',
        ],
    );
    assert.deepEqual(
        manifest.contributes.views['splitscript-debugger'].map(view => view.name),
        ['Runtime', 'Statistics', 'Settings', 'Settings Map', 'Variables', 'Processes'],
    );
    assert.deepEqual(
        manifest.contributes.views['splitscript-debugger'].map(view => view.contextualTitle),
        ['Runtime', 'Statistics', 'Settings', 'Settings Map', 'Variables', 'Processes'],
    );
    assert(manifest.contributes.viewsWelcome.some(
        welcome => welcome.view === 'splitscript.debugger.runtime'
            && welcome.when === '!splitscript.debug.active',
    ));
    const actions = manifest.contributes.menus['view/title']
        .filter(item => item.when?.includes('view == splitscript.debugger.runtime'))
        .map(item => item.command);
    assert.deepEqual(actions, [
        'splitscript.debug.timerStart',
        'splitscript.debug.timerReset',
        'splitscript.debug.restart',
        'splitscript.debug.stop',
        'splitscript.debug.showLogs',
    ]);
    assert(manifest.contributes.menus['view/title'].some(
        item => item.command === 'splitscript.debug.clearSettings'
            && item.when?.includes('view == splitscript.debugger.settingsMap'),
    ));
    const statisticsActions = manifest.contributes.menus['view/title']
        .filter(item => item.when?.includes('view == splitscript.debugger.statistics'))
        .map(item => item.command);
    assert.deepEqual(statisticsActions, [
        'splitscript.debug.resetStatistics',
        'splitscript.debug.openMemory',
    ]);
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

test('debug watch does not add a second play action to SplitScript editor titles', () => {
    const editorTitleCommands = manifest.contributes.menus['editor/title']
        .map(item => item.command);
    assert(!editorTitleCommands.includes('splitscript.startDebugWatch'));
    assert(!editorTitleCommands.includes('splitscript.stopDebugWatch'));
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

test('formatter policy can inherit editorconfig or be overridden per workspace', () => {
    const properties = manifest.contributes.configuration.properties;
    assert.equal(properties['splitscript.formatting.useEditorConfig'].default, true);
    for (const name of [
        'maxLineWidth',
        'indentStyle',
        'indentWidth',
        'lineEnding',
        'insertFinalNewline',
    ]) {
        assert.equal(properties[`splitscript.formatting.${name}`].default, null);
    }
});

test('documentation gives enum variants their dedicated palette color', () => {
    assert.match(
        documentationStyles,
        /\[data-splitscript-token="enumMember"\]\s*\{\s*color:\s*#F397FF;/,
    );
});
