-- Run from repository root: lua scripts/test-plugin.lua
package.path = 'lightroom/SonyPixelShift.lrplugin/?.lua;' .. package.path
local p = require 'Protocol'
local json = require 'Json'
local engine = require 'Engine'
assert(p.jsonString('a"b\\c\n') == '"a\\"b\\\\c\\u000a"')
assert(p.quote("a'b", false) == "'a'\\''b'")
assert(p.quote('C:\\User Files\\engine.exe', true) == '"C:\\User Files\\engine.exe"')
assert(p.quote('C:\\Program Files (x86)\\a&b.exe', true) == '"C:\\Program Files (x86)\\a&b.exe"')
for _, bad in ipairs({'%TEMP%', 'a!b', 'a^b', 'a"b', 'a\nb'}) do
    assert(not pcall(p.quote, bad, true))
end
assert(engine.command('C:\\User Files\\engine.exe', {'merge', '--job', 'C:\\写真\\job.json'}, true) ==
    '""C:\\User Files\\engine.exe" "merge" "--job" "C:\\写真\\job.json""')
local job = json.decode(p.job({'a"b.ARW', 'C:\\写真\\x.ARW'}, 'b.tif', 'four'))
assert(job.camera == "auto")
assert(job.version == 1 and job.inputs[1] == 'a"b.ARW' and job.inputs[2] == 'C:\\写真\\x.ARW')
assert(json.decode('"\\uD83D\\uDC3B"') == '\240\159\144\187')
assert(json.decode('{"null":null,"false":false,"n":-1.25e2}').n == -125)
for _, bad in ipairs({'{"x":1,"x":2}', '[1,]', '{"x":}', '01', '1.', '1.e2', '1e400', 'true false', '"\\uD800"', '"\\uDC00"', '"a\nb"'}) do
    assert(not pcall(json.decode, bad), 'Accepted invalid JSON: ' .. bad)
end
local manifest = {protocol_version=1, engine_version='0.1.0'}
engine.checkManifest(manifest)
engine.checkInfo({protocol_version=1, engine_version='0.1.0',camera_models={'ILCE-7CR'},autodetection=true}, manifest)
assert(not pcall(engine.checkManifest, {protocol_version=2,engine_version='0.1.0'}))
assert(not pcall(engine.checkInfo, {protocol_version=1,engine_version='9.0',camera_models={'ILCE-7CR'},autodetection=true}, manifest))
local info = dofile('lightroom/SonyPixelShift.lrplugin/Info.lua')
assert(#info.LrLibraryMenuItems == 2)
print('Plugin protocol checks passed')
dofile('scripts/test-plugin-workflow.lua')
