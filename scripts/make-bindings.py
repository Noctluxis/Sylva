# SPDX-License-Identifier: MPL-2.0
"""Write our action manifest from the supported physical control semantics."""
import json
from pathlib import Path
root = Path(__file__).resolve().parent.parent / 'bindings'
root.mkdir(exist_ok=True)
def write(name, value):
    (root / name).write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')
actions = [{'name': '/actions/main/in/' + n, 'type': 'boolean', 'requirement': 'mandatory'} for n in ['click','grab','toggle']]
actions.append({'name':'/actions/main/out/haptic','type':'vibration','requirement':'optional'})
write('actions.json', {
    'version': 2, 'minimum_required_version': 1,
    'action_sets': [{'name':'/actions/main','usage':'leftright'}],
    'actions':actions,
    'default_bindings':[{'controller_type':c,'binding_url':c+'.json'} for c in ['oculus_touch','pico_controller']],
    'localization':[{'language_tag':'en_us','/actions/main':'Sylva keyboard','/actions/main/in/click':'Click key','/actions/main/in/grab':'Grab panel','/actions/main/in/toggle':'Toggle panel (double B)'}]
})
sources=[]
for hand in ['left','right']:
    for control,action in [('trigger','click'),('grip','grab')]:
        sources.append({'path':f'/user/hand/{hand}/input/{control}','mode':'button','parameters':{'click_activation_threshold':'0.7','click_deactivation_threshold':'0.55'},'inputs':{'click':{'output':f'/actions/main/in/{action}'}}})
sources.append({'path':'/user/hand/right/input/b','mode':'button','inputs':{'double':{'output':'/actions/main/in/toggle'}}})
for controller in ['oculus_touch','pico_controller']:
    write(controller+'.json', {'action_manifest_version':2,'controller_type':controller,'name':'Sylva — dual pointers','description':'Trigger: click. Grip: move. Double B: show/hide.','category':'steamvr_input','bindings':{'/actions/main':{'sources':sources,'haptics':[{'path':f'/user/hand/{hand}/output/haptic','output':'/actions/main/out/haptic'} for hand in ['left','right']]}}})
