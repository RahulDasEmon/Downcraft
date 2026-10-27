import numpy as np, colorsys, os
from PIL import Image
from scipy import ndimage as ndi
SRC='fullhd_src'; OUT='out'; os.makedirs(OUT,exist_ok=True)
MAP={'delete':'8830284c-08c2-4428-a012-8e3742cc5079','options':'Blue Gear Hawk Emblem','stop_all':'Fierce Eagle Hand Emblem',
'delete_completed':'Fierce Red Shield Eagle Emblem','friend':'Glossy Blue Hawk Emblem','scheduler':'Glossy Eagle Alarm Badge',
'start_queue':'Glossy Eagle Download Folder Logo','resume':'Glossy Hawk Emblem with Downward Arrow','add':'Glossy Neon Hawk Cross Emblem',
'stop_queue':'Neon Purple Falcon Emblem','stop':'Purple Hawk Esports Emblem'}
CANVAS=192; CONTENT=180; DESAT=0.75
def fade(rgb):
    f=(rgb/255.0).reshape(-1,3); mx=f.max(1); mn=f.min(1); v=mx; s=np.where(mx>0,(mx-mn)/np.maximum(mx,1e-9),0)
    s2=s*DESAT; # keep hue & value: new min = v*(1-s2)
    newmn=v*(1-s2); k=np.where((mx-mn)>1e-9,(v-newmn)/np.maximum(mx-mn,1e-9),0)
    out=np.where((mx-mn)[:,None]>1e-9,v[:,None]-(v[:,None]-f)*k[:,None],f)
    return (out.reshape(rgb.shape)*255)
for name,fn in MAP.items():
    im=np.array(Image.open(f'{SRC}/{fn}.png').convert('RGBA')).astype(np.float32)
    a=im[...,3]/255
    ys,xs=np.where(a>0.01); y0,y1,x0,x1=ys.min(),ys.max()+1,xs.min(),xs.max()+1
    a=a[y0:y1,x0:x1]; c=im[y0:y1,x0:x1,:3]
    h,w=a.shape; k=CONTENT/max(h,w); nw,nh=round(w*k),round(h*k)
    pm=np.dstack([c*a[...,None],a*255]).astype(np.float32)   # premultiplied -> no dark/light fringe
    pm=np.dstack([np.array(Image.fromarray(pm[...,j],mode='F').resize((nw,nh),Image.LANCZOS)) for j in range(4)])
    A=np.clip(pm[...,3]/255,0,1)
    rgb=np.clip(np.where(A[...,None]>1e-4,pm[...,:3]/np.maximum(A,1e-4)[...,None],0),0,255)
    _,(by,bx)=ndi.distance_transform_edt(~(A>0.02),return_indices=True); rgb=fade(rgb[by,bx])
    fr=np.zeros((CANVAS,CANVAS,3),np.float32); fa=np.zeros((CANVAS,CANVAS),np.float32)
    oy,ox=(CANVAS-nh)//2,(CANVAS-nw)//2
    fr[oy:oy+nh,ox:ox+nw]=rgb; fa[oy:oy+nh,ox:ox+nw]=A*255
    m=np.zeros((CANVAS,CANVAS),bool); m[oy:oy+nh,ox:ox+nw]=True
    _,(cy,cx)=ndi.distance_transform_edt(~m,return_indices=True); fr=fr[cy,cx]
    Image.fromarray(np.dstack([fr,fa]).round().astype(np.uint8),'RGBA').save(f'{OUT}/{name}.png',optimize=True)
    print(name,(nw,nh))
