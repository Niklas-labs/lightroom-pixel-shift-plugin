-- Exercise the files users install, with no package.path/source-tree fallback.
-- Only the standard globals needed by the plugin are supplied; toolkit code
-- must not depend on metatables, package, loadfile or the test runner's globals.
local root = assert(arg[1], 'Usage: lua test-plugin-load.lua path/to/Plugin.lrplugin')
local cache, queued = {}, {}
local scope = {ended=false}
function scope:setCancelable(value) assert(value == false) end
function scope:setPausable(pause,cancel) assert(pause == false and cancel == false) end
function scope:setIndeterminate() assert(not self.ended) end
function scope:setCaption(value) self.caption=value end
function scope:setPortionComplete(done,total) assert(done >= 0 and total > 0) end
function scope:isDone() return self.ended end
function scope:done() self.ended=true end
local tasks = {startAsyncTask=function(fn) queued[#queued+1]=fn end}
-- SDK namespaces are explicit: unknown imports must fail like Lightroom.
-- Object types such as LrObservableTable are not importable namespaces.
local sdk = {
    LrShell={}, LrApplication={}, LrDialogs={}, LrFileUtils={}, LrFunctionContext={},
    LrPathUtils={}, LrPrefs={}, LrView={}, LrBinding={},
    LrTasks=tasks, LrProgressScope=function() return scope end,
}
local base = {assert=assert,error=error,ipairs=ipairs,pairs=pairs,type=type,
    tonumber=tonumber,tostring=tostring,select=select,unpack=unpack,
    string=string,table=table,math=math,io=io,WIN_ENV=false,_PLUGIN={path=root}}
local toolkitRequire
local function loadToolkit(name)
    assert(name:match('^[%w_]+$'), 'Invalid toolkit script name')
    local path = root .. '/' .. name .. '.lua'
    local chunk,problem=loadfile(path)
    assert(chunk, 'Cannot load packaged toolkit script ' .. path .. ': ' .. tostring(problem))
    local env={}
    for key,value in pairs(base) do env[key]=value end
    env.require=toolkitRequire
    env.import=function(key)
        return assert(sdk[key], 'Unknown Lightroom SDK namespace: ' .. tostring(key))
    end
    setfenv(chunk,env)
    return chunk()
end
toolkitRequire=function(name)
    if cache[name] == nil then cache[name]=loadToolkit(name) or true end
    return cache[name]
end
local info=toolkitRequire('Info')
assert(info.LrExportMenuItems == nil and #info.LrLibraryMenuItems == 2)
for _,item in ipairs(info.LrLibraryMenuItems) do
    toolkitRequire(assert(item.file:match('^(.*)%.lua$')))
end
assert(#queued == 2, 'Packaged menu scripts did not reach the workflow')
local progress=toolkitRequire('PixelShiftProgress').new(scope,4,tasks)
assert(scope.caption == 'Preparing')
progress:line('decode 1/4: source.ARW')
assert(scope.caption == 'Decoding RAW 1 of 4')
progress:line('merge and motion correction')
assert(scope.caption == 'Aligning and merging')
progress:done()
assert(scope.ended)
print('Packaged Lightroom toolkit load and progress construction passed')
