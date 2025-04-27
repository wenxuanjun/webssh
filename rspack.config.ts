import { defineConfig } from '@rspack/cli'
import { rspack } from '@rspack/core'
import WasmPackPlugin from '@wasm-tool/wasm-pack-plugin'
import path from 'path'

export default defineConfig({
    entry: './index.js',
    output: {
        path: path.resolve('dist'),
        filename: 'index.js',
    },
    plugins: [
        new rspack.HtmlRspackPlugin({
            template: 'index.html',
        }),
        new WasmPackPlugin({
            crateDirectory: path.resolve('.'),
            // extraArgs: '--profiling --no-opt',
        }),
    ],
    mode: 'production',
    experiments: {
        asyncWebAssembly: true
    }
})
