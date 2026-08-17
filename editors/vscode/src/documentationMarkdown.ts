/**
 * Restricts language-server Markdown to the one command used by compiler-owned
 * reference links. Other command URIs remain inert.
 */
export const documentationMarkdownTrust = {
    isTrusted: {
        enabledCommands: ['splitscript.openDocumentation'],
    },
} as const;
