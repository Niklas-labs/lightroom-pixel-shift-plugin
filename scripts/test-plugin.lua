-- Run from repository root: lua scripts/test-plugin.lua
package.path = 'lightroom/SonyPixelShift.lrplugin/?.lua;' .. package.path
local p = require 'Protocol'
assert(p.jsonString('a"b\\c\n') == '"a\\"b\\\\c\\u000a"')
assert(p.quote("a'b", false) == "'a'\\''b'")
assert(p.quote('C:\\User Files\\engine.exe', true) == '"C:\\User Files\\engine.exe"')
for _, bad in ipairs({'%TEMP%', 'a!b', 'a&b', 'a"b', 'a\nb'}) do
    assert(not pcall(p.quote, bad, true))
end
assert(p.job({'a.ARW'}, 'b.tif', 'four'):find('"version":1', 1, true))
local info = dofile('lightroom/SonyPixelShift.lrplugin/Info.lua')
assert(#info.LrLibraryMenuItems == 2)
print('Plugin protocol checks passed')
