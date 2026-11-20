-- Exercise the installed menu entry, including expanded stacks and changes at the write gate.
local cases = 0
local function run(flags, selection, options)
    cases=cases+1; options=options or {}
    local state={flags={},writes={},errors={},bezels={},gates=0,queue={}}
    local before, photos, members = {}, {}, {}
    for id,value in pairs(flags) do state.flags[id]=value; before[id]=value end
    for id in pairs(flags) do
        local p={localIdentifier=id}
        function p:getRawMetadata(key)
            if options.readError and id==5 then error('metadata unavailable') end
            if key=='isInStackInFolder' then return id<=6 end
            if key=='stackInFolderMembers' then
                if state.inWrite and options.membershipChanged and id<=3 then return {photos[1],photos[2],photos[7]} end
                return members[id<=3 and 1 or 2]
            end
            if key=='countStackInFolderMembers' then return 3 end
            if key=='stackInFolderIsCollapsed' then
                if options.unknownCollapse and id<=3 then return nil end
                if options.inconsistentCollapse and id==2 then return false end
                if state.inWrite and options.expandedDuringWait and id>=4 then return false end
                return id<=3 and not options.expanded or id>=4 and not options.expandedSecond
            end
            if key=='stackPositionInFolder' then
                if options.badPosition and id==3 then return 2 end
                if state.inWrite and options.reorderedDuringWait and id<=2 then return 3-id end
                if options.newTop and id<=2 then return 3-id end
                return (id-1)%3+1
            end
            if key=='pickStatus' then return state.flags[id] end
            error('Unexpected metadata key: ' .. key)
        end
        function p:setRawMetadata(key,value)
            assert(state.inWrite and key=='pickStatus' and value==-1)
            state.writes[#state.writes+1]=id;state.flags[id]=value
            if options.writeError and #state.writes==2 then error('write failed') end
        end
        photos[id]=p
    end
    members[1]={photos[3],photos[1],photos[2]}
    if photos[4] then members[2]={photos[6],photos[4],photos[5]} end
    local selected={}
    for _,id in ipairs(selection) do selected[#selected+1]=photos[id] end
    local catalog={}
    function catalog:getTargetPhoto() return selected[1] end
    function catalog:getTargetPhotos() return #selected==0 and {photos[1]} or selected end
    function catalog:getActiveSources()
        return {{type=function() return options.collection and 'LrCollection' or 'LrFolder' end}}
    end
    function catalog:withWriteAccessDo(title, callback, params)
        assert(title=='Reject hidden photos in collapsed stacks' and params.timeout==30)
        state.gates=state.gates+1
        if options.timeout then return 'aborted' end
        state.inWrite=true
        local ok, problem=pcall(callback)
        state.inWrite=false
        if not ok then
            for id,value in pairs(before) do state.flags[id]=value end -- SDK transaction rollback.
            error(problem)
        end
        return 'executed'
    end
    local sdk={LrApplication={activeCatalog=function() return catalog end},
        LrTasks={pcall=pcall,startAsyncTask=function(fn) state.queue[#state.queue+1]=fn end},
        LrDialogs={showBezel=function(text) state.bezels[#state.bezels+1]=text end,
            message=function(_,text) state.errors[#state.errors+1]=text end}}
    local env=setmetatable({import=function(key) return assert(sdk[key],key) end}, {__index=_G})
    local chunk=assert(loadfile('lightroom/SonyPixelShift.lrplugin/StackFlags.lua'));setfenv(chunk,env)
    local module=chunk()
    local entry=assert(loadfile('lightroom/SonyPixelShift.lrplugin/RejectHiddenStackPhotos.lua'))
    setfenv(entry,{require=function(key) assert(key=='StackFlags');return module end});entry()
    assert(#state.queue==1);state.queue[1]()
    state.before=before
    return state
end
local function expect(state, flags)
    assert(#state.errors==0 and #state.bezels==1)
    local count,seen=0,{}
    for id,value in pairs(flags) do
        assert(state.flags[id]==value,'Unexpected flag on photo ' .. id)
        if state.before[id]~=value then count=count+1 end
    end
    assert(#state.writes==count and state.gates==(count>0 and 1 or 0))
    for _,id in ipairs(state.writes) do assert(not seen[id]);seen[id]=true end
end
for _,top in ipairs({-1,0,1}) do
    for _,a in ipairs({-1,0,1}) do
        for _,b in ipairs({-1,0,1}) do
            for _,expanded in ipairs({false,true}) do
                for _,selection in ipairs({{1},{3,2,1,3}}) do
                    local state=run({[1]=top,[2]=a,[3]=b},selection,{expanded=expanded})
                    expect(state,{[1]=top,[2]=expanded and a or -1,[3]=expanded and b or -1})
                    for _,id in ipairs(state.writes) do assert(id~=1,'Top must never be written') end
                end
            end
        end
    end
end
expect(run({[1]=0,[2]=0,[3]=1,[4]=1,[5]=0,[6]=-1,[7]=1},{1,4,7},{expandedSecond=true}),
    {[1]=0,[2]=-1,[3]=-1,[4]=1,[5]=0,[6]=-1,[7]=1})
expect(run({[1]=0,[2]=1,[3]=0},{3},{newTop=true}),{[1]=-1,[2]=1,[3]=-1})
expect(run({[1]=0,[2]=0,[3]=0,[7]=0},{7}),{[1]=0,[2]=0,[3]=0,[7]=0})
for _,option in ipairs({'unknownCollapse','inconsistentCollapse','badPosition','collection','timeout',
    'writeError','readError','expandedDuringWait','reorderedDuringWait','membershipChanged','empty'}) do
    local flags={[1]=0,[2]=0,[3]=1,[4]=1,[5]=0,[6]=0,[7]=0}
    local state=run(flags,option=='empty' and {} or {1,4},{[option]=true})
    assert(#state.errors==1 and #state.bezels==0,option)
    for id,value in pairs(flags) do assert(state.flags[id]==value,option .. ': partial change') end
    if option~='writeError' then assert(#state.writes==0,option) end
end
print('Collapsed-stack rejection checks passed (' .. cases .. ' cases)')
