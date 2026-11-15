import numpy as np, spiceypy as sp, sys
L='/usr/local/lib/python3.13/dist-packages/lunarsky/data/'
for k in ['/usr/local/lib/python3.13/dist-packages/skyfield_data/data/de421.bsp',L+'pck/moon_pa_de421_1900-2050.bpc',
          L+'fk/satellites/moon_080317.tf',L+'fk/satellites/moon_assoc_me.tf','naif_lsk.tls']: sp.furnsh(k)
R=1737.4
def enu(lat,lon):
    la,lo=np.radians(lat),np.radians(lon)
    up=np.array([np.cos(la)*np.cos(lo),np.cos(la)*np.sin(lo),np.sin(la)])
    east=np.array([-np.sin(lo),np.cos(lo),0.0]); north=np.cross(up,east)
    return up,east,north
def track(lat,lon,utc0,days,step_h=1.0,abcorr='LT+S'):
    et0=sp.str2et(utc0); ets=et0+np.arange(0,days*24,step_h)*3600
    up,e,n=enu(lat,lon); site=up*R
    out={}
    for b in ('SUN','EARTH'):
        al=[];az=[]
        for et in ets:
            v,_=sp.spkpos(b,et,'MOON_ME',abcorr,'MOON'); v=np.array(v)-site
            al.append(np.degrees(np.arcsin(v@up/np.linalg.norm(v)))); az.append(np.degrees(np.arctan2(v@e,v@n))%360)
        out[b]=(np.array(al),np.array(az))
    return ets,out
if __name__=="__main__":
    t,o=track(-89.9,0,"2027-01-01",365,3)
    print("lat 89.9S: Sun alt min/max %.2f %.2f"%(o['SUN'][0].min(),o['SUN'][0].max()))
    print("lat 89.9S: Earth alt min/max %.2f %.2f"%(o['EARTH'][0].min(),o['EARTH'][0].max()))
    t,o=track(-89.54,129.2,"2027-01-01",60,1)
    s,e=o['SUN'][0],o['EARTH'][0]
    print("approx Shackleton-rim pt, flat horizon, 60 d: sun up %.1f%%  earth up %.1f%%  both %.1f%%"%((s>0).mean()*100,(e>0).mean()*100,((s>0)&(e>0)).mean()*100))
