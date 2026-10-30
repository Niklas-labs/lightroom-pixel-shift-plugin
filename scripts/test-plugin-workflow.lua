-- Mock only the Lightroom SDK boundary; run the actual plugin workflow and IO.
package.path = 'lightroom/SonyPixelShift.lrplugin/?.lua;' .. package.path
local Protocol = require 'Protocol'
local Json = require 'Json'
local function taskPcall(fn, ...)
    local task = coroutine.create(fn)
    local args = {...}
    while true do
        local result = {coroutine.resume(task, unpack(args))}
        if not result[1] or coroutine.status(task) == 'dead' then return unpack(result) end
        args = {coroutine.yield(unpack(result, 2))}
    end
end
local function scheduler(queue)
    local turns = 0
    while #queue > 0 do
        local task = table.remove(queue,1)
        local ok, problem = coroutine.resume(task)
        assert(ok, problem)
        if coroutine.status(task) ~= 'dead' then queue[#queue+1] = task end
        turns = turns + 1
        assert(turns < 10000, 'Workflow failed to complete')
    end
end
local function append(path, text)
    local f = assert(io.open(path,'ab')); assert(f:write(text)); assert(f:close())
end
local base = os.tmpname()
os.remove(base)
assert(os.execute('mkdir -p ' .. Protocol.quote(base .. '/plugin/bin', false)) == 0)
local function write(path, text)
    local f = assert(io.open(path, 'wb')); assert(f:write(text)); assert(f:close())
end
local function exists(path)
    local f = io.open(path, 'rb')
    if f then f:close(); return 'file' end
end
local function read(path)
    local f = assert(io.open(path, 'rb')); local s = f:read('*a'); f:close(); return s
end
local function cleanup()
    os.execute('rm -rf ' .. Protocol.quote(base, false))
end
local ok, problem = pcall(function()
    write(base .. '/plugin/bin/sony-pixelshift', 'mock executable')
    write(base .. '/plugin/manifest.json', '{"protocol_version":1,"engine_version":"0.1.0"}')
    for i=1,16 do write(base .. '/DSC' .. string.format('%05d', i) .. '.ARW', 'mock raw') end
    for _, scenario in ipairs({'success', 'disk', 'sixteen', 'denoise', 'options-cancel', 'filtered-selection', 'selection-error', 'unconfirmed-import', 'engine-failure', 'execute-error', 'launch-failure', 'bad-handshake', 'import-failure', 'catalog-timeout', 'wrong-output'}) do
        local state = {imports={},dialogs={},commands={}, registry={}, queue={}, captions={}}
        local denoise = scenario == 'denoise'
        local count = scenario == 'sixteen' and 16 or 4
        local output
        local function photo(path) return {getRawMetadata=function(_, key) assert(key == 'path'); return path end} end
        local catalog = {}
        function catalog:getTargetPhotos()
            local photos = {}
            for i=1,count do
                local path = base .. '/DSC' .. string.format('%05d', i) .. '.ARW'
                photos[i] = {getRawMetadata=function(_, key) assert(key == 'path'); return path end}
            end
            return photos
        end
        function catalog:withWriteAccessDo(_, callback)
            if scenario == 'catalog-timeout' then return 'aborted' end
            state.writeContext = true; callback(); state.writeContext = false
            return 'executed'
        end
        function catalog:addPhoto(path)
            assert(state.writeContext, 'Import occurred outside catalog write context')
            assert(exists(path) == 'file', 'Imported output was missing')
            if scenario == 'import-failure' then error('catalog import denied') end
            state.imports[#state.imports+1] = path
            local imported = photo(path)
            if scenario ~= 'unconfirmed-import' then state.registry[path] = imported end
            return imported
        end
        function catalog:findPhotoByPath(path) return state.registry[path] end
        function catalog:setSelectedPhotos(active, others)
            assert(not state.writeContext and #others == 0)
            if scenario == 'selection-error' then error('selection blocked') end
            if scenario ~= 'filtered-selection' then state.selected = active end
        end
        function catalog:getTargetPhoto() return state.selected end
        function catalog:setViewFilter() error('Library filters must be preserved') end
        function catalog:setActiveSources() error('Library sources must be preserved') end
        local sdk = {
            LrApplication={activeCatalog=function() return catalog end},
            LrProgressScope=function(options)
                assert(state.context and options.functionContext == state.contextObject)
                local scope = {ended=false, closeCount=0}
                state.scope = scope
                function scope:setCancelable(value) assert(value == false) end
                function scope:setPausable(pausable,cancelable) assert(pausable == false and cancelable == false) end
                function scope:setIndeterminate() assert(not self.ended) end
                function scope:setPortionComplete(done,total)
                    assert(not self.ended and done >= 0 and done < total and (total == count or total == 3))
                end
                function scope:setCaption(caption)
                    assert(not self.ended)
                    state.captions[#state.captions+1] = {text=caption, duringExecute=state.inExecute}
                end
                function scope:isDone() return self.ended end
                function scope:done()
                    assert(not self.ended, 'Scope closed more than once')
                    self.ended=true; self.closeCount=self.closeCount+1
                end
                return scope
            end,
            LrObservableTable={makePropertyTable=function() return {} end},
            LrView={bind=function(key) return key end,osFactory=function() return {control_spacing=function() return 8 end,static_text=function(_, t) return t end,row=function(_,t) return t end,popup_menu=function(_,t) return t end,column=function(_, t) return t end} end},
            LrDialogs={presentModalDialog=function(options)
                state.options = options
                if scenario == 'disk' then options.contents.bind_to.sourceStorage = 'disk' end
                assert(options.contents[2].title:find(denoise and 'Less noise' or (count == 16 and 'Higher resolution' or 'Better color detail'), 1, true))
                return scenario == 'options-cancel' and 'cancel' or 'ok'
            end,runSavePanel=function() error('Unexpected output picker') end,
                message=function(title,message,severity) assert(not state.scope or state.scope.ended, 'Progress remained active during dialog'); state.dialogs[#state.dialogs+1]={title=title,message=message,severity=severity} end},
            LrFileUtils={exists=exists,chooseUniqueFileName=function(path)
                local candidate, n = path, 1
                while exists(candidate) do
                    n = n + 1
                    local stem, ext = path:match('^(.*)(%.[^%.]+)$')
                    candidate = stem .. '-' .. n .. ext
                end
                if path:match('%.dng$') then output = candidate end
                return candidate
            end,
                delete=function(path) os.remove(path) end},
            LrFunctionContext={callWithContext=function(_, callback)
                state.context=true; state.contextObject={}
                local ok,problem=taskPcall(callback,state.contextObject)
                if state.scope and not state.scope.ended then state.scope:done() end
                state.context=false
                if not ok then error(problem,0) end
            end},
            LrPathUtils={child=function(a,b) return a .. '/' .. b end,parent=function(a) return a:match('^(.*)/') end,leafName=function(a) return a:match('([^/]+)$') end},
            LrPrefs={prefsForPlugin=function() return {} end},
            LrTasks={startAsyncTask=function(callback) state.queue[#state.queue+1]=coroutine.create(callback) end,
                pcall=taskPcall,sleep=function() coroutine.yield() end,yield=function() coroutine.yield() end,
                execute=function(command)
                    assert(state.context, 'Process ran outside function context')
                    state.commands[#state.commands+1]=command
                    if command:find("'info'",1,true) then
                        if scenario == 'launch-failure' then return 126 end
                        local path=assert(command:match("'%-?%-output' '([^']+)'"))
                        write(path, '{"protocol_version":1,"engine_version":"' ..
                            (scenario == 'bad-handshake' and '9.0' or '0.1.0') .. '","camera_models":["ILCE-7CR"],"autodetection":true,"output_formats":["dng","tiff"],"source_storages":["ram","disk"],"streamed_output":true,"output_compressions":["lossless","none"]}')
                        return 0
                    end
                    state.job=assert(command:match("'%-%-job' '([^']+)'"))
                    state.result=assert(command:match("'%-%-result' '([^']+)'"))
                    state.log=assert(command:match("'%-%-log' '([^']+)'"))
                    local job=Json.decode(read(state.job)); assert(job.source_storage == (scenario == 'disk' and 'disk' or 'ram')); assert(job.mode == (denoise and 'denoise' or 'auto') and job.camera == 'auto'); assert(job.inputs[1] == base .. '/DSC00001.ARW')
                    if scenario == 'execute-error' then error('process launch raised an exception') end
                    state.inExecute = true
                    for i=1,count do
                        append(state.log, 'decode ' .. i .. '/' .. count .. ': source.ARW\n')
                        coroutine.yield()
                    end
                    append(state.log, 'merge and motion correction\n')
                    coroutine.yield()
                    append(state.log, 'writing 24x16 DNG\n')
                    coroutine.yield()
                    if scenario == 'engine-failure' then
                        write(state.result, '{"version":1,"status":"error","message":"test capture rejected"}')
                        state.inExecute=false
                        return 1
                    end
                    append(state.log, 'output region 1/2\n')
                    coroutine.yield()
                    append(state.log, 'output region 2/2\n')
                    coroutine.yield()
                    write(output, 'mock DNG')
                    write(state.result, '{"version":1,"status":"complete","output":' ..
                        Protocol.jsonString(scenario == 'wrong-output' and output .. '.wrong' or output) .. '}')
                    append(state.log, 'complete in 1.0s; 0 pixels used reference fallback\n')
                    state.inExecute=false
                    return 0
                end},
        }
        _G.import=function(name) return assert(sdk[name], name) end
        _G._PLUGIN={path=base .. '/plugin'}
        _G.WIN_ENV=false
        package.loaded.Workflow=nil
        require('Workflow').start(denoise)
        scheduler(state.queue)
        if scenario == 'options-cancel' then assert(not state.scope)
        else assert(state.scope.ended and state.scope.closeCount == 1) end
        if scenario == 'success' or scenario == 'disk' or scenario == 'sixteen' or scenario == 'denoise' or scenario == 'filtered-selection' or scenario == 'selection-error' then
            local seen = {}
            for _, update in ipairs(state.captions) do
                if update.duringExecute then seen[update.text]=true end
            end
            assert(seen['Decoding RAW 1 of ' .. count] and seen['Aligning and merging'] and seen['Writing DNG'] and seen['Combining and saving 2 of 2'], 'No live progress during execution')
            assert(#state.imports == 1 and state.imports[1] == output)
            assert(not exists(state.job) and not exists(state.result))
            assert(exists(state.log) == 'file')
            assert(state.dialogs[1].severity == 'info')
            assert(state.dialogs[1].message:find('Catalog: Confirmed', 1, true))
            if scenario == 'filtered-selection' or scenario == 'selection-error' then
                assert(state.dialogs[1].message:find('Check library filters', 1, true))
            else assert(state.selected == state.registry[output]) end
            assert(output:find(denoise and 'NoiseReduced' or 'PixelShift', 1, true))
        elseif scenario == 'options-cancel' then
            assert(#state.commands == 0 and #state.imports == 0 and #state.dialogs == 0)
        else
            assert(#state.imports == (scenario == 'unconfirmed-import' and 1 or 0))
            assert(state.dialogs[1].severity == 'critical')
            assert(state.dialogs[1].message:find('Diagnostic log:',1,true))
            local jobPath=assert(state.job or output .. '.pixelshift-job.json')
            assert(exists(jobPath) == 'file', 'Failed job was not retained')
            assert(not exists(jobPath .. '.engine.json'), 'Handshake result was not cleaned')
            if scenario == 'launch-failure' then
                assert(state.dialogs[1].message:find('Privacy & Security',1,true))
                assert(#state.commands == 1)
            end
            if scenario == 'engine-failure' then
                assert(state.dialogs[1].message:find('test capture rejected',1,true))
            end
        end
    end
    local Output = require 'Output'
    local pathUtils = { parent=function(p) return p:match('^(.*)/') end,
        leafName=function(p) return p:match('([^/]+)$') end,
        child=function(a,b) return a .. '/' .. b end }
    local identity = function(p) return p end
    assert(Output.path({'/raw/DSC01585.ARW', '/raw/DSC01582.ARW'}, false, pathUtils, identity) == '/raw/DSC01582-DSC01585-PixelShift.dng')
    assert(Output.path({'/raw/one.ARW', '/raw/two.ARW'}, true, pathUtils, identity) == '/raw/one-two-NoiseReduced.dng')
    assert(not pcall(Output.path, {'/a/one.ARW', '/b/two.ARW'}, false, pathUtils, identity))
    for i=1,16 do assert(read(base .. '/DSC' .. string.format('%05d', i) .. '.ARW') == 'mock raw') end
end)
cleanup()
assert(ok, problem)
print('Plugin mocked workflow checks passed (15 scenarios, with cooperative engine/progress scheduling)')
