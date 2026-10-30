-- Mock only the Lightroom SDK boundary; run the actual plugin workflow and IO.
package.path = 'lightroom/SonyPixelShift.lrplugin/?.lua;' .. package.path
local Protocol = require 'Protocol'
local Json = require 'Json'
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
    for _, scenario in ipairs({'success', 'sixteen', 'denoise', 'options-cancel', 'filtered-selection', 'selection-error', 'unconfirmed-import', 'engine-failure', 'launch-failure', 'bad-handshake', 'import-failure', 'catalog-timeout', 'wrong-output'}) do
        local state = {imports={},dialogs={},commands={}, registry={}}
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
            LrView={osFactory=function() return {control_spacing=function() return 8 end,static_text=function(_, t) return t end,column=function(_, t) return t end} end},
            LrDialogs={presentModalDialog=function(options)
                state.options = options
                assert(options.contents[2].title:find(denoise and 'Less noise' or (count == 16 and 'Higher resolution' or 'Better color detail'), 1, true))
                return scenario == 'options-cancel' and 'cancel' or 'ok'
            end,runSavePanel=function() error('Unexpected output picker') end,
                message=function(title,message,severity) state.dialogs[#state.dialogs+1]={title=title,message=message,severity=severity} end},
            LrFileUtils={exists=exists,chooseUniqueFileName=function(path)
                local candidate, n = path, 1
                while exists(candidate) do
                    n = n + 1
                    local stem, ext = path:match('^(.*)(%.[^%.]+)$')
                    candidate = stem .. '-' .. n .. ext
                end
                if path:match('%.tif$') then output = candidate end
                return candidate
            end,
                delete=function(path) os.remove(path) end},
            LrFunctionContext={callWithContext=function(_, callback) state.context=true; callback({}); state.context=false end},
            LrPathUtils={child=function(a,b) return a .. '/' .. b end,parent=function(a) return a:match('^(.*)/') end,leafName=function(a) return a:match('([^/]+)$') end},
            LrPrefs={prefsForPlugin=function() return {} end},
            LrTasks={startAsyncTask=function(callback) callback() end,pcall=pcall,sleep=function() end,yield=function() end,
                execute=function(command)
                    assert(state.context, 'Process ran outside function context')
                    state.commands[#state.commands+1]=command
                    if command:find("'info'",1,true) then
                        if scenario == 'launch-failure' then return 126 end
                        local path=assert(command:match("'%-?%-output' '([^']+)'"))
                        write(path, '{"protocol_version":1,"engine_version":"' ..
                            (scenario == 'bad-handshake' and '9.0' or '0.1.0') .. '","camera_models":["ILCE-7CR"],"autodetection":true}')
                        return 0
                    end
                    state.job=assert(command:match("'%-%-job' '([^']+)'"))
                    state.result=assert(command:match("'%-%-result' '([^']+)'"))
                    state.log=assert(command:match("'%-%-log' '([^']+)'"))
                    local job=Json.decode(read(state.job)); assert(job.mode == (denoise and 'denoise' or 'auto') and job.camera == 'auto'); assert(job.inputs[1] == base .. '/DSC00001.ARW')
                    write(state.log, 'mock engine diagnostics')
                    if scenario == 'engine-failure' then
                        write(state.result, '{"version":1,"status":"error","message":"test capture rejected"}')
                        return 1
                    end
                    write(output, 'mock TIFF')
                    write(state.result, '{"version":1,"status":"complete","output":' ..
                        Protocol.jsonString(scenario == 'wrong-output' and output .. '.wrong' or output) .. '}')
                    return 0
                end},
        }
        _G.import=function(name) return assert(sdk[name], name) end
        _G._PLUGIN={path=base .. '/plugin'}
        _G.WIN_ENV=false
        package.loaded.Workflow=nil
        require('Workflow').start(denoise)
        if scenario == 'success' or scenario == 'sixteen' or scenario == 'denoise' or scenario == 'filtered-selection' or scenario == 'selection-error' then
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
    assert(Output.path({'/raw/DSC01585.ARW', '/raw/DSC01582.ARW'}, false, pathUtils, identity) == '/raw/DSC01582-DSC01585-PixelShift.tif')
    assert(Output.path({'/raw/one.ARW', '/raw/two.ARW'}, true, pathUtils, identity) == '/raw/one-two-NoiseReduced.tif')
    assert(not pcall(Output.path, {'/a/one.ARW', '/b/two.ARW'}, false, pathUtils, identity))
    for i=1,16 do assert(read(base .. '/DSC' .. string.format('%05d', i) .. '.ARW') == 'mock raw') end
end)
cleanup()
assert(ok, problem)
print('Plugin mocked workflow checks passed (13 scenarios)')
