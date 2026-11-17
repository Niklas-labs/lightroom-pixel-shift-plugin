-- Exercise the real menu command, including reordered stacks and mixed flags.
package.path = 'lightroom/SonyPixelShift.lrplugin/?.lua;' .. package.path
local cases = 0
local function run(flags, selection, options)
    options = options or {}
    cases = cases+1
    local before, photos, members = {}, {}, {}
    local state = { flags={}, writes={}, bezels={}, errors={}, queue={}, gates=0 }
    for id, value in pairs(flags) do state.flags[id]=value; before[id]=value end
    for id in pairs(flags) do
        local p = {localIdentifier=id, isVirtualCopy=options.newTop and id==2}
        function p:getRawMetadata(key)
            assert(not state.inWrite, 'Metadata must be read before writing')
            if options.lateError and id>=4 then error('late metadata failure') end
            if key=='isInStackInFolder' then return id<=6 end
            if key=='stackInFolderMembers' then return members[id<=3 and 1 or 2] end
            if key=='countStackInFolderMembers' then return 3 end
            if key=='stackPositionInFolder' then
                if options.missingPosition and id==3 then return nil end
                if options.duplicatePosition and id==3 then return 2 end
                if options.newTop and id==1 then return 2 end
                if options.newTop and id==2 then return 1 end
                return (id-1)%3+1
            end
            if key=='pickStatus' then return state.flags[id] end
            error('Unexpected metadata key: ' .. key)
        end
        function p:setRawMetadata(key,value)
            assert(state.inWrite and key=='pickStatus' and (value==-1 or value==0 or value==1))
            state.writes[#state.writes+1]={id=id,value=value}
            state.flags[id]=value
            if options.writeError and #state.writes==2 then error('injected sync write failure') end
        end
        photos[id]=p
    end
    -- Deliberately unordered. Selection/array order must never determine the top.
    members[1]={photos[3],photos[1],photos[2]}
    if photos[4] then members[2]={photos[6],photos[5],photos[4]} end
    if options.overlap then members[2]={photos[3],photos[5],photos[4]} end
    local selected={}
    for _,id in ipairs(selection) do selected[#selected+1]=photos[id] end
    local catalog={}
    function catalog:getActiveSources()
        return {{type=function() return options.collection and 'LrCollection' or 'LrFolder' end}}
    end
    function catalog:getTargetPhoto() return selected[1] end
    function catalog:getTargetPhotos()
        -- Match the SDK's dangerous empty-selection fallback.
        if #selected==0 then return {photos[1],photos[2],photos[3]} end
        return selected
    end
    function catalog:setSelectedPhotos() error('Selection must be preserved') end
    function catalog:setViewFilter() error('Filters must be preserved') end
    function catalog:withWriteAccessDo(title, callback, params)
        assert(title=='Sync rejection from stack top' and params.timeout==30 and not params.asynchronous)
        state.gates=state.gates+1
        if options.timeout then return 'aborted' end
        state.inWrite=true
        local ok,problem=pcall(callback)
        state.inWrite=false
        if not ok then
            -- Lightroom rolls back all metadata writes when the callback fails.
            for id,value in pairs(before) do state.flags[id]=value end
            error(problem,0)
        end
        return 'executed'
    end
    local sdk={LrApplication={activeCatalog=function() return catalog end},
        LrTasks={pcall=pcall,startAsyncTask=function(fn) state.queue[#state.queue+1]=fn end},
        LrDialogs={showBezel=function(message) assert(not state.inWrite);state.bezels[#state.bezels+1]=message end,
            message=function(title,message,severity)
                assert(title=='Stack flags' and severity=='critical');state.errors[#state.errors+1]=message
            end}}
    _G.import=function(name) return assert(sdk[name],name) end
    package.loaded.StackFlags=nil
    dofile('lightroom/SonyPixelShift.lrplugin/SyncStackRejection.lua')
    assert(#state.queue==1)
    state.queue[1]()
    state.before=before
    return state
end
local function expect(state, expected)
    assert(#state.errors==0 and #state.bezels==1)
    local count, seen=0,{}
    for id,value in pairs(expected) do
        assert(state.flags[id]==value,'Wrong flag for photo ' .. id)
        if value~=state.before[id] then count=count+1 end
    end
    assert(#state.writes==count and state.gates==(count>0 and 1 or 0))
    for _,write in ipairs(state.writes) do
        assert(not seen[write.id], 'Duplicate write');seen[write.id]=true
        assert(state.before[write.id]~=write.value,'Unnecessary write')
    end
end
for _,top in ipairs({-1,0,1}) do
    for _,a in ipairs({-1,0,1}) do
        for _,b in ipairs({-1,0,1}) do
            for _,selection in ipairs({{1},{3,2,1,3}}) do
                local function desired(value)
                    if top==-1 then return -1 end
                    return value==-1 and 0 or value
                end
                local state=run({[1]=top,[2]=a,[3]=b},selection)
                expect(state,{[1]=top,[2]=desired(a),[3]=desired(b)})
                assert(state.flags[1]==top, 'Top was modified')
                for _,write in ipairs(state.writes) do assert(write.id~=1,'Top was written') end
            end
        end
    end
end
-- Each selected stack follows its own top; unstacked photos stay untouched.
expect(run({[1]=-1,[2]=0,[3]=1,[4]=0,[5]=-1,[6]=1,[7]=-1},{2,5,1,7}),
    {[1]=-1,[2]=-1,[3]=-1,[4]=0,[5]=0,[6]=1,[7]=-1})
-- A reordered top can itself be a virtual copy; only actual position matters.
expect(run({[1]=-1,[2]=1,[3]=-1},{1},{newTop=true}),{[1]=0,[2]=1,[3]=0})
local none=run({[1]=-1,[2]=0,[3]=1,[7]=-1},{7})
expect(none,{[1]=-1,[2]=0,[3]=1,[7]=-1})
assert(none.bezels[1]:find('No folder stacks',1,true))
local aligned=run({[1]=-1,[2]=-1,[3]=-1},{1})
expect(aligned,{[1]=-1,[2]=-1,[3]=-1})
assert(aligned.bezels[1]:find('already matches',1,true))
for _,option in ipairs({'missingPosition','duplicatePosition','collection','timeout','writeError','lateError','overlap','badTop','badMember','empty'}) do
    local flags={[1]=-1,[2]=0,[3]=1,[4]=0,[5]=-1,[6]=1}
    local options={[option]=true}
    if option=='badTop' then flags[1]=2 end
    if option=='badMember' then flags[2]=2 end
    local state=run(flags,option=='empty' and {} or {1,4},options)
    assert(#state.errors==1 and #state.bezels==0,option)
    for id,value in pairs(state.before) do assert(state.flags[id]==value,option .. ': partial write') end
    if option~='writeError' then assert(#state.writes==0,option) end
    assert(state.gates==((option=='timeout' or option=='writeError') and 1 or 0),option)
end
print('Stack rejection sync checks passed (' .. cases .. ' cases)')
