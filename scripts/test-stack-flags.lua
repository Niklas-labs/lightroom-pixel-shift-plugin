-- Run actual menu entry scripts and stack-flag code against the documented SDK boundary.
package.path = 'lightroom/SonyPixelShift.lrplugin/?.lua;' .. package.path
local cases = {
    'collapsed', 'siblings', 'multiple-stacks', 'multiple-folders', 'unstacked', 'mixed', 'virtual-copy', 'virtual-member',
    'empty', 'no-selection-filmstrip', 'collection', 'mixed-sources', 'special-source', 'no-source',
    'missing-members', 'missing-count', 'incomplete-members', 'duplicate-members',
    'wrong-members', 'metadata-error', 'late-metadata-error', 'write-error', 'timeout', 'invalid-status',
}
local total = 0
for _, status in ipairs({1,-1,0}) do
    for _, scenario in ipairs(cases) do
        total = total+1
        local state = {writes={},queue={},bezel={},errors={},reads=0,gates=0,flags={},selected={}}
        local function photo(id, inStack)
            local p = {localIdentifier=id}
            state.flags[id] = id % 3 - 1
            function p:getRawMetadata(key)
                assert(not state.inWrite, 'Stack membership was read during the write gate')
                state.reads=state.reads+1
                if scenario == 'metadata-error' or (scenario == 'late-metadata-error' and id>=4) then error('metadata unavailable') end
                if key == 'isInStackInFolder' then return inStack end
                if key == 'stackInFolderMembers' then
                    if scenario == 'virtual-member' then return {state.a,state.b,state.virtual} end
                    if scenario == 'missing-members' then return nil end
                    if scenario == 'incomplete-members' then return {state.a,state.b} end
                    if scenario == 'duplicate-members' then return {state.a,state.b,state.b} end
                    if scenario == 'wrong-members' then return {state.d,state.e,state.f} end
                    return id < 4 and {state.a,state.b,state.c} or {state.d,state.e,state.f}
                end
                if key == 'countStackInFolderMembers' then
                    if scenario == 'missing-count' then return nil end
                    return 3
                end
                error('Unexpected metadata read: ' .. key)
            end
            function p:setRawMetadata(key,value)
                assert(state.inWrite and key == 'pickStatus' and value == status)
                state.writes[id]=(state.writes[id] or 0)+1
                state.flags[id]=value
                if scenario == 'write-error' and id == 2 then error('injected catalog write failure') end
            end
            return p
        end
        state.a,state.b,state.c = photo(1,true),photo(2,true),photo(3,true)
        state.d,state.e,state.f = photo(4,true),photo(5,true),photo(6,true)
        state.single=photo(7,false)
        state.virtual=photo(8,false) -- distinct LrPhoto: no expansion to a master or other copies
        local folder = {type=function() return 'LrFolder' end}
        local collection = {type=function() return 'LrCollection' end}
        local sources = {folder}
        local expected = {1,2,3}
        state.selected={state.a} -- collapsed top only; b/c hidden by filters or collapse
        if scenario == 'siblings' then
            -- Separate proxies with the same identifier must not cause duplicate writes.
            state.selected={state.a,photo(2,true),state.c,state.a}
        elseif scenario == 'multiple-stacks' or scenario == 'multiple-folders' then
            state.selected={state.b,state.e}; expected={1,2,3,4,5,6}
            if scenario == 'multiple-folders' then sources={folder,{type=function() return 'LrFolder' end}} end
        elseif scenario == 'late-metadata-error' then state.selected={state.a,state.d}
        elseif scenario == 'virtual-member' then state.selected={state.a};expected={1,2,8}
        elseif scenario == 'unstacked' then state.selected={state.single}; expected={7}
        elseif scenario == 'mixed' then state.selected={state.c,state.single}; expected={1,2,3,7}
        elseif scenario == 'virtual-copy' then state.selected={state.virtual}; expected={8}
        elseif scenario == 'empty' or scenario == 'no-selection-filmstrip' then state.selected={}
        elseif scenario == 'collection' then sources={collection}
        elseif scenario == 'mixed-sources' then sources={folder,collection}
        elseif scenario == 'special-source' then sources={'allPhotos'}
        elseif scenario == 'no-source' then sources={} end
        local before={}
        for id,value in pairs(state.flags) do before[id]=value end
        local catalog={}
        function catalog:getActiveSources() return sources end
        function catalog:getTargetPhoto() return state.selected[1] end
        function catalog:getTargetPhotos()
            if scenario == 'no-selection-filmstrip' then return {state.a,state.single} end
            return state.selected
        end
        function catalog:setSelectedPhotos() error('Must not change selection') end
        function catalog:setViewFilter() error('Must not change filters') end
        function catalog:withWriteAccessDo(name,callback,options)
            state.gates=state.gates+1
            assert(options.timeout == 30 and not options.asynchronous)
            assert(name == ({[1]='Flag entire stack',[-1]='Reject entire stack',[0]='Unflag entire stack'})[status])
            if scenario == 'timeout' then return 'aborted' end
            state.inWrite=true
            local ok,problem=pcall(callback)
            state.inWrite=false
            if not ok then
                -- SDK contract: an exception rolls back all writes in this gate.
                for id,value in pairs(before) do state.flags[id]=value end
                error(problem,0)
            end
            return 'executed'
        end
        local sdk={
            LrApplication={activeCatalog=function() return catalog end},
            LrTasks={startAsyncTask=function(fn) state.queue[#state.queue+1]=fn end,pcall=pcall},
            LrDialogs={
                showBezel=function(message,delay) assert(not state.inWrite and delay>0);state.bezel[#state.bezel+1]=message end,
                message=function(title,message,severity) assert(title=='Stack flags' and severity=='critical');state.errors[#state.errors+1]=message end,
            },
        }
        _G.import=function(name) return assert(sdk[name], 'Unexpected namespace: ' .. name) end
        package.loaded.StackFlags=nil
        if scenario == 'invalid-status' then require('StackFlags').start(9)
        else
            local entry=({[1]='FlagStack',[-1]='RejectStack',[0]='UnflagStack'})[status]
            dofile('lightroom/SonyPixelShift.lrplugin/' .. entry .. '.lua')
        end
        assert(#state.queue==1)
        state.queue[1]()
        local success = scenario=='collapsed' or scenario=='siblings' or scenario=='multiple-stacks' or
            scenario=='multiple-folders' or scenario=='virtual-member' or
            scenario=='unstacked' or scenario=='mixed' or scenario=='virtual-copy'
        if success then
            assert(#state.errors==0 and #state.bezel==1 and state.gates==1)
            local expectedSet={}
            for _,id in ipairs(expected) do expectedSet[id]=true; assert(state.flags[id]==status and state.writes[id]==1) end
            for id,value in pairs(before) do if not expectedSet[id] then assert(state.flags[id]==value and not state.writes[id]) end end
            assert(state.bezel[1]:find(tostring(#expected) .. (#expected==1 and ' photo' or ' photos'),1,true))
        else
            assert(#state.errors==1 and #state.bezel==0,scenario)
            for id,value in pairs(before) do assert(state.flags[id]==value,scenario .. ': partial update') end
            if scenario~='write-error' then assert(next(state.writes)==nil,scenario) end
            assert(state.gates==((scenario=='write-error' or scenario=='timeout') and 1 or 0))
            if scenario=='timeout' then assert(state.errors[1]:find('Catalog busy',1,true)) end
            if scenario=='collection' then assert(state.errors[1]:find('Folders panel',1,true)) end
            -- Errors must release the module's busy guard, so a retry can work.
            state.selected={state.single};sources={folder}
            local failedScenario=scenario;scenario='unstacked'
            require('StackFlags').start(status)
            state.queue[2]()
            assert(#state.bezel==1,failedScenario .. ': retry did not run')
        end
    end
end
-- Force a second command to run while the first is in its write gate.
do
    local inside=false
    local n=0
    local queue,bezel,errors={},{},{}
    local p={localIdentifier=1,getRawMetadata=function() return false end,
        setRawMetadata=function(_,key,value) assert(inside and key=='pickStatus' and value==1);n=n+1 end}
    local catalog={getActiveSources=function() return {{type=function() return 'LrFolder' end}} end,
        getTargetPhoto=function() return p end,getTargetPhotos=function() return {p} end}
    function catalog:withWriteAccessDo(_,callback)
        inside=true
        queue[2]() -- second task must decline without changing flags
        callback();inside=false;return 'executed'
    end
    local sdk={LrApplication={activeCatalog=function() return catalog end},
        LrTasks={pcall=pcall,startAsyncTask=function(fn) queue[#queue+1]=fn end},
        LrDialogs={showBezel=function(s) bezel[#bezel+1]=s end,message=function(s) errors[#errors+1]=s end}}
    _G.import=function(k) return assert(sdk[k]) end
    package.loaded.StackFlags=nil
    require('StackFlags').start(1); require('StackFlags').start(-1)
    queue[1]()
    assert(n==1 and #errors==0 and #bezel==2 and bezel[1]:find('updating',1,true))
end
print('Stack flag checks passed (' .. total .. ' cases plus overlapping-command guard)')
