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
    for i=1,4 do write(base .. '/photo-' .. i .. '.ARW', 'mock raw') end
    for _, scenario in ipairs({'success', 'cancel', 'engine-failure', 'bad-handshake', 'import-failure', 'catalog-timeout', 'wrong-output'}) do
        local state = {imports={},dialogs={},commands={}}
        local output = base .. '/' .. scenario .. ' Output.tif'
        local catalog = {}
        function catalog:getTargetPhotos()
            local photos = {}
            for i=1,4 do
                local path = base .. '/photo-' .. i .. '.ARW'
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
            return {}
        end
        local sdk = {
            LrApplication={activeCatalog=function() return catalog end},
            LrDialogs={runSavePanel=function() if scenario ~= 'cancel' then return output end end,
                message=function(title,message,severity) state.dialogs[#state.dialogs+1]={title=title,message=message,severity=severity} end},
            LrFileUtils={exists=exists,chooseUniqueFileName=function(path) assert(not exists(path)); return path end,
                delete=function(path) os.remove(path) end},
            LrFunctionContext={callWithContext=function(_, callback) state.context=true; callback({}); state.context=false end},
            LrPathUtils={child=function(a,b) return a .. '/' .. b end,parent=function(a) return a:match('^(.*)/') end},
            LrPrefs={prefsForPlugin=function() return {} end},
            LrTasks={startAsyncTask=function(callback) callback() end,pcall=pcall,
                execute=function(command)
                    assert(state.context, 'Process ran outside function context')
                    state.commands[#state.commands+1]=command
                    if command:find("'info'",1,true) then
                        local path=assert(command:match("'%-?%-output' '([^']+)'"))
                        write(path, '{"protocol_version":1,"engine_version":"' ..
                            (scenario == 'bad-handshake' and '9.0' or '0.1.0') .. '","camera_models":["ILCE-7CR"]}')
                        return 0
                    end
                    state.job=assert(command:match("'%-%-job' '([^']+)'"))
                    state.result=assert(command:match("'%-%-result' '([^']+)'"))
                    state.log=assert(command:match("'%-%-log' '([^']+)'"))
                    local job=Json.decode(read(state.job)); assert(job.inputs[1] == base .. '/photo-1.ARW')
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
        require('Workflow').start(false)
        if scenario == 'success' then
            assert(#state.imports == 1 and state.imports[1] == output)
            assert(not exists(state.job) and not exists(state.result))
            assert(exists(state.log) == 'file')
            assert(state.dialogs[1].severity == 'info')
        elseif scenario == 'cancel' then
            assert(#state.commands == 0 and #state.imports == 0 and #state.dialogs == 0)
        else
            assert(#state.imports == 0)
            assert(state.dialogs[1].severity == 'critical')
            assert(state.dialogs[1].message:find('Diagnostic log:',1,true))
            local jobPath=output .. '.pixelshift-job.json'
            assert(exists(jobPath) == 'file', 'Failed job was not retained')
            assert(not exists(jobPath .. '.engine.json'), 'Handshake result was not cleaned')
            if scenario == 'engine-failure' then
                assert(state.dialogs[1].message:find('test capture rejected',1,true))
            end
        end
    end
end)
cleanup()
assert(ok, problem)
print('Plugin mocked workflow checks passed (7 scenarios)')
