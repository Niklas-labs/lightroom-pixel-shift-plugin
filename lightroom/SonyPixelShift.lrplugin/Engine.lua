local Protocol = require 'Protocol'
local Json = require 'Json'
local M = { protocolVersion = 1, version = '0.1.0' }
function M.readJson(path)
    local file, problem = io.open(path, 'rb')
    assert(file, 'Cannot read ' .. path .. ': ' .. tostring(problem))
    local text = file:read('*a')
    file:close()
    return Json.decode(text)
end
function M.executable(pluginPath, windows, preferences, exists, child)
    if preferences.useCustomEngine then
        assert(type(preferences.enginePath) == 'string' and exists(preferences.enginePath) == 'file', 'The developer engine override is missing. Disable the override or update its path.')
        return preferences.enginePath, true
    end
    local executable = child(child(pluginPath, 'bin'), windows and 'sony-pixelshift.exe' or 'sony-pixelshift')
    assert(exists(executable) == 'file', 'The bundled engine is missing. Download and extract the complete plugin artifact for your operating system.')
    return executable, false
end
function M.checkManifest(manifest)
    assert(type(manifest) == 'table' and manifest.protocol_version == M.protocolVersion, 'The plugin package has an incompatible protocol version. Reinstall the complete plugin artifact.')
    assert(manifest.engine_version == M.version, 'The plugin package contains a different engine version. Reinstall the complete plugin artifact.')
end
function M.checkInfo(info, manifest)
    assert(type(info) == 'table' and info.protocol_version == M.protocolVersion, 'The engine uses an incompatible plugin protocol.')
    assert(info.engine_version == M.version, 'The engine and plugin versions do not match.')
    if manifest then assert(info.engine_version == manifest.engine_version, 'The engine does not match its package manifest.') end
    assert(type(info.camera_models) == 'table' and #info.camera_models > 0, 'The engine has no supported camera profiles.')
    local supportsDng = false
    for _, format in ipairs(info.output_formats or {}) do
        if format == 'dng' then supportsDng = true end
    end
    local supportsLossless = false
    for _, compression in ipairs(info.output_compressions or {}) do
        if compression == 'lossless' then supportsLossless = true end
    end
    assert(info.streamed_output == true and supportsLossless, 'This engine lacks streamed lossless output. Reinstall the complete current plugin package.')
    assert(supportsDng, 'This engine cannot write DNG. Reinstall the complete current plugin package.')
    assert(info.autodetection == true, 'This engine does not support automatic capture detection. Reinstall the complete current plugin package.')
end
function M.userError(value)
    local message = tostring(value)
    message = message:gsub('^%[string "[^"]+"%]:%d+:%s*', '')
    message = message:gsub('^.-%.lua:%d+:%s*', '')
    return message
end
function M.launchError(status, windows)
    local message = 'Engine launch failed (exit ' .. tostring(status) .. ').\nCheck: correct platform package'
    if not windows then
        message = message .. '\nmacOS: Privacy & Security > Open Anyway\nManual fix: see TESTING.md > macOS execution'
    end
    return message
end
function M.command(executable, args, windows)
    local parts = { Protocol.quote(executable, windows) }
    for _, value in ipairs(args) do parts[#parts + 1] = Protocol.quote(value, windows) end
    local command = table.concat(parts, ' ')
    return windows and ('"' .. command .. '"') or command
end
return M
