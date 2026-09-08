const BIGINT_RESULT_IMPORTS = new Set([
    'env.process_attach',
    'env.process_attach_by_pid',
    'env.process_get_memory_range_address',
    'env.process_get_memory_range_count',
    'env.process_get_memory_range_flags',
    'env.process_get_memory_range_size',
    'env.process_get_module_address',
    'env.process_get_module_size',
    'env.setting_value_copy',
    'env.setting_value_new_bool',
    'env.setting_value_new_f64',
    'env.setting_value_new_i64',
    'env.setting_value_new_list',
    'env.setting_value_new_map',
    'env.setting_value_new_string',
    'env.settings_list_copy',
    'env.settings_list_len',
    'env.settings_list_new',
    'env.settings_map_copy',
    'env.settings_map_len',
    'env.settings_map_load',
    'env.settings_map_new',
]);

/** Returns a type-correct neutral result for an unavailable ASR import. */
export function neutralImport(qualifiedName: string): () => number | bigint {
    return BIGINT_RESULT_IMPORTS.has(qualifiedName) ? () => 0n : () => 0;
}
