//! Sky Glance: a glanceable weather dashboard for Mira glasses.
//! The phone companion fetches Open-Meteo data and sends compact text records;
//! this module parses them into fixed buffers and draws four swipeable tabs.
#![no_std]
#![allow(static_mut_refs)]
#[path = "../Shared/companion.rs"] mod companion;
#[path = "../Shared/content.rs"] mod content;
#[path = "../Shared/text.rs"] mod metrics;

// Sun peeking out from behind a cloud.
app_icon!(content::app_icon::Icon::new()
    .circle(11,10,5).line(11,1,11,2).line(2,10,3,10).line(4,3,5,4).line(18,3,17,4).line(19,10,20,10)
    .circle(19,20,7).circle(10,22,5).line(6,27,28,27).0);

#[link(wasm_import_module="mira")]
extern "C" {
    fn begin(); fn present();
    fn text(x:i32,y:i32,w:i32,h:i32,c:i32,font:i32,a:i32,s:i32,p:*const u8,n:i32);
    fn fill_rect(x:i32,y:i32,w:i32,h:i32,c:i32);
    fn stroke_rect(x:i32,y:i32,w:i32,h:i32,l:i32,c:i32);
    fn fill_round_rect(x:i32,y:i32,w:i32,h:i32,r:i32,c:i32);
    fn fill_circle(x:i32,y:i32,r:i32,c:i32);
    fn stroke_circle(x:i32,y:i32,r:i32,l:i32,c:i32);
    fn line(x0:i32,y0:i32,x1:i32,y1:i32,w:i32,c:i32);
    fn fill_triangle(x0:i32,y0:i32,x1:i32,y1:i32,x2:i32,y2:i32,c:i32);
    fn foreground_interval_ms(ms:i32)->i32;
}
#[panic_handler] fn panic(_: &core::panic::PanicInfo)->! { loop {} }

const APP:&[u8]=b"Sky Glance";
const TABS:usize=4;
const GRID:usize=7; const CELLS:usize=GRID*GRID; const MAP_HOURS:usize=12;
const HOURS:usize=24;

// ---------------------------------------------------------------- state
static mut BRIDGE:companion::Companion<3600,128>=companion::Companion::new(APP);
static mut TAB:usize=0;
static mut DIRTY:bool=true;
static mut NEED_SYNC:bool=true;
static mut NEED_REFRESH:bool=false;
static mut REFRESHING:bool=false;
static mut WAIT_MS:u32=0;

static mut CUR:[u8;640]=[0;640]; static mut CUR_N:usize=0;
static mut STATUS:[u8;120]=[0;120]; static mut STATUS_N:usize=0;

#[derive(Clone,Copy)]
struct Hour { label:[u8;4], ll:u8, temp:i16, pop:u8, amt:u16, sun:u8, uv:u8, wind:u8, dir:u16, gust:u8 }
const NO_HOUR:Hour=Hour{label:[0;4],ll:0,temp:0,pop:0,amt:0,sun:0,uv:0,wind:0,dir:0,gust:0};
static mut HRS:[Hour;HOURS]=[NO_HOUR;HOURS]; static mut HRS_N:usize=0;
static mut H_PAGE:usize=0;

static mut M_N:usize=0;
static mut M_P:[[u8;CELLS];MAP_HOURS]=[[0;CELLS];MAP_HOURS];   // precip level 0..9
static mut M_D:[[u8;CELLS];MAP_HOURS]=[[0;CELLS];MAP_HOURS];   // wind-from sector 0..15
static mut M_S:[[u8;CELLS];MAP_HOURS]=[[0;CELLS];MAP_HOURS];   // wind mph
static mut M_LABEL:[[u8;4];MAP_HOURS]=[[0;4];MAP_HOURS]; static mut M_LL:[u8;MAP_HOURS]=[0;MAP_HOURS];
static mut M_FRAME:usize=0; static mut M_PLAY:bool=true; static mut M_ACC:u32=0;

// ---------------------------------------------------------------- parsing
fn find(h:&[u8],n:&[u8])->Option<usize>{
    if n.len()>h.len(){return None;}
    (0..=h.len()-n.len()).find(|&i| &h[i..i+n.len()]==n)
}
fn json_str<'a>(m:&'a[u8],key:&[u8])->&'a[u8]{
    match find(m,key){
        Some(i)=>{let s=i+key.len();let e=m[s..].iter().position(|&b|b==b'"').map(|p|s+p).unwrap_or(m.len());&m[s..e]}
        None=>&[],
    }
}
fn nth(s:&[u8],sep:u8,i:usize)->&[u8]{ s.split(|&b|b==sep).nth(i).unwrap_or(&[]) }
fn int(s:&[u8])->i32{
    let (neg,d)=if s.first()==Some(&b'-'){(true,&s[1..])}else{(false,s)};
    let mut v:i32=0; for &b in d { if !b.is_ascii_digit(){break;} v=v.saturating_mul(10).saturating_add((b-b'0') as i32); }
    if neg{-v}else{v}
}
fn b36(c:u8)->u8{ match c { b'0'..=b'9'=>c-b'0', b'a'..=b'z'=>c-b'a'+10, _=>0 } }
fn label4(s:&[u8])->([u8;4],u8){ let mut l=[0u8;4]; let n=s.len().min(4); l[..n].copy_from_slice(&s[..n]); (l,n as u8) }

unsafe fn set_status(s:&[u8]){ let n=s.len().min(STATUS.len()); STATUS[..n].copy_from_slice(&s[..n]); STATUS_N=n; }

unsafe fn handle(m:&[u8]){
    let k=json_str(m,b"\"k\":\""); let d=json_str(m,b"\"d\":\"");
    match k {
        b"c"=>{ let n=d.len().min(CUR.len()); CUR[..n].copy_from_slice(&d[..n]); CUR_N=n; REFRESHING=false; STATUS_N=0; }
        b"h"=>parse_hours(d),
        b"m"=>parse_map(d),
        b"s"=>{ set_status(d); if find(d,b"rror").is_some()||find(d,b"ailed").is_some(){REFRESHING=false;} }
        _=>return,
    }
    DIRTY=true;
}
unsafe fn parse_hours(d:&[u8]){
    let mut n=0;
    for e in d.split(|&b|b==b';') {
        if n>=HOURS || e.is_empty(){break;}
        let (label,ll)=label4(nth(e,b',',0));
        HRS[n]=Hour{label,ll,temp:int(nth(e,b',',1)) as i16,pop:int(nth(e,b',',2)).clamp(0,100) as u8,
            amt:int(nth(e,b',',3)).clamp(0,9999) as u16,sun:int(nth(e,b',',4)).clamp(0,100) as u8,
            uv:int(nth(e,b',',5)).clamp(0,250) as u8,wind:int(nth(e,b',',6)).clamp(0,250) as u8,
            dir:int(nth(e,b',',7)).rem_euclid(360) as u16,gust:int(nth(e,b',',8)).clamp(0,250) as u8};
        n+=1;
    }
    HRS_N=n;
}
unsafe fn parse_map(d:&[u8]){
    if int(nth(d,b'|',0)) as usize!=GRID {return;}
    let hours=(int(nth(d,b'|',1)).max(0) as usize).min(MAP_HOURS);
    let labels=nth(d,b'|',2); let cells=nth(d,b'|',3);
    if cells.len()<hours*CELLS*3 {return;}
    for h in 0..hours {
        let (l,ll)=label4(nth(labels,b',',h)); M_LABEL[h]=l; M_LL[h]=ll;
        for i in 0..CELLS {
            let at=(h*CELLS+i)*3;
            M_P[h][i]=cells[at].wrapping_sub(b'0').min(9);
            M_D[h][i]=cells[at+1].wrapping_sub(b'a').min(15);
            M_S[h][i]=b36(cells[at+2])*2;
        }
    }
    M_N=hours; if M_FRAME>=M_N {M_FRAME=0;}
}
unsafe fn cf(i:usize)->&'static [u8]{ nth(&CUR[..CUR_N],b'|',i) }
unsafe fn ci(i:usize)->i32{ int(cf(i)) }
// Current-conditions fields, in the order the companion sends them.
const C_LOC:usize=0; const C_COND:usize=1; const C_UPDATED:usize=2; const C_TEMP:usize=3; const C_FEELS:usize=4;
const C_HUM:usize=5; const C_HI:usize=6; const C_LO:usize=7; const C_WIND:usize=8; const C_WDIR:usize=9;
const C_GUST:usize=10; const C_CODE:usize=11; const C_DAY:usize=12; const C_DEW:usize=13; #[allow(dead_code)] const C_UV:usize=14;
const C_RAIN:usize=15; const C_RISE:usize=16; const C_SET:usize=17; const C_POP:usize=18;

// ---------------------------------------------------------------- text helpers
struct Buf{b:[u8;200],n:usize}
impl Buf{
    const fn new()->Self{Self{b:[0;200],n:0}}
    fn add(&mut self,s:&[u8])->&mut Self{let n=s.len().min(200-self.n);self.b[self.n..self.n+n].copy_from_slice(&s[..n]);self.n+=n;self}
    fn num(&mut self,v:i32)->&mut Self{
        let mut t=[0u8;12];let mut i=12;let mut x=(v as i64).abs();
        loop{i-=1;t[i]=b'0'+(x%10) as u8;x/=10;if x==0{break;}}
        if v<0{i-=1;t[i]=b'-';}
        self.add(&t[i..])
    }
    fn hundredths(&mut self,v:u16)->&mut Self{ self.num((v/100) as i32); self.add(b"."); if v%100<10{self.add(b"0");} self.num((v%100) as i32) }
    fn s(&self)->&[u8]{&self.b[..self.n]}
}

fn line_h(font:i32)->i32{ match font {1=>24,2=>47,3=>57,_=>33} }
fn fw(font:i32,s:&[u8])->i32{ if font==1 {metrics::width(0,s)*3/4} else {metrics::width(font,s)} }
fn fit(font:i32,s:&[u8],max:i32)->&[u8]{
    if fw(font,s)<=max {return s;}
    let mut n=s.len();
    while n>0 && (fw(font,&s[..n])>max || (n<s.len() && s[n]&0xc0==0x80)) {n-=1;}
    &s[..n]
}
unsafe fn tx(x:i32,y:i32,w:i32,ink:i32,font:i32,align:i32,s:&[u8]){
    if s.is_empty(){return;}
    let s=fit(font,s,w);
    text(x,y,w,line_h(font),ink,font,align,line_h(font),s.as_ptr(),s.len() as i32);
}
unsafe fn column(x:i32,y:i32,w:i32,ink:i32,font:i32,align:i32,spacing:i32,s:&[u8]){
    if s.is_empty(){return;}
    text(x,y,w,spacing*13,ink,font,align,spacing,s.as_ptr(),s.len() as i32);
}
/// Text followed by a drawn degree ring (the firmware font set is ASCII-safe).
unsafe fn deg(x:i32,y:i32,ink:i32,font:i32,s:&[u8])->i32{
    tx(x,y,360,ink,font,0,s);
    let w=fw(font,s); let r=match font{3=>5,2=>4,_=>3};
    let cy=y+match font{3=>12,2=>10,1=>6,_=>8};
    stroke_circle(x+w+r+3,cy,r,2,ink);
    x+w+2*r+8
}
const COMPASS:[&[u8];16]=[b"N",b"NNE",b"NE",b"ENE",b"E",b"ESE",b"SE",b"SSE",b"S",b"SSW",b"SW",b"WSW",b"W",b"WNW",b"NW",b"NNW"];
fn sector(deg:i32)->usize{ ((deg.rem_euclid(360)*2+22)/45 % 16) as usize }

// ---------------------------------------------------------------- drawing primitives
const SIN16:[i32;16]=[0,383,707,924,1000,924,707,383,0,-383,-707,-924,-1000,-924,-707,-383];
/// Arrow centred on (cx,cy) pointing toward compass sector `s` (0 = north/up).
unsafe fn arrow(cx:i32,cy:i32,s:usize,len:i32,ink:i32,w:i32){
    let ux=SIN16[s%16]; let uy=-SIN16[(s+4)%16];
    let hx=cx+ux*len/2000; let hy=cy+uy*len/2000;
    let tx0=cx-ux*len/2000; let ty0=cy-uy*len/2000;
    let hl=(len*2/5).clamp(6,18); let hw=hl*6/10;
    let bx=hx-ux*hl/1000; let by=hy-uy*hl/1000;
    line(tx0,ty0,bx,by,w,ink);
    fill_triangle(hx,hy,bx-uy*hw/1000,by+ux*hw/1000,bx+uy*hw/1000,by-ux*hw/1000,ink);
}
const SEG:[u8;11]=[0x3f,0x06,0x5b,0x4f,0x66,0x6d,0x7d,0x07,0x7f,0x6f,0x40];
/// Seven-segment digit; index 10 is a minus sign.
unsafe fn digit(x:i32,y:i32,w:i32,h:i32,t:i32,d:usize,ink:i32){
    let m=SEG[d.min(10)]; let hh=h/2; let r=t/2; let g=2;
    let vh=hh-t/2-2*g;
    if m&1!=0 {fill_round_rect(x+t/2+g,y,w-t-2*g,t,r,ink);}
    if m&2!=0 {fill_round_rect(x+w-t,y+t/2+g,t,vh,r,ink);}
    if m&4!=0 {fill_round_rect(x+w-t,y+hh+g,t,vh,r,ink);}
    if m&8!=0 {fill_round_rect(x+t/2+g,y+h-t,w-t-2*g,t,r,ink);}
    if m&16!=0 {fill_round_rect(x,y+hh+g,t,vh,r,ink);}
    if m&32!=0 {fill_round_rect(x,y+t/2+g,t,vh,r,ink);}
    if m&64!=0 {fill_round_rect(x+t/2+g,y+hh-t/2,w-t-2*g,t,r,ink);}
}
/// Large temperature readout; returns the x just past the degree ring.
unsafe fn big_temp(x:i32,y:i32,v:i32)->i32{
    let (w,h,t,gap)=(54,104,12,14);
    let mut cx=x;
    if v<0 { digit(cx,y,w,h,t,10,15); cx+=w+gap; }
    let a=v.unsigned_abs().min(199);
    let mut ds=[0usize;3]; let mut n=0; let mut q=a;
    loop{ ds[n]=(q%10) as usize; n+=1; q/=10; if q==0{break;} }
    for i in (0..n).rev(){ digit(cx,y,w,h,t,ds[i],15); cx+=w+gap; }
    stroke_circle(cx+8,y+12,9,4,15);
    cx+26
}
unsafe fn sun(cx:i32,cy:i32,r:i32,ink:i32){
    fill_circle(cx,cy,r,ink);
    for k in 0..8 { let s=k*2; let (ux,uy)=(SIN16[s],-SIN16[(s+4)%16]);
        line(cx+ux*(r+6)/1000,cy+uy*(r+6)/1000,cx+ux*(r+15)/1000,cy+uy*(r+15)/1000,3,ink); }
}
unsafe fn moon(cx:i32,cy:i32,r:i32){ fill_circle(cx,cy,r,14); fill_circle(cx+r/2,cy-r/3,r*4/5,0); }
unsafe fn cloud(cx:i32,cy:i32,ink:i32){
    fill_circle(cx-18,cy+6,16,ink); fill_circle(cx+4,cy-6,22,ink); fill_circle(cx+24,cy+8,14,ink);
    fill_round_rect(cx-34,cy+6,72,22,11,ink);
}
/// Condition icon from the WMO weather code.
unsafe fn weather_icon(cx:i32,cy:i32,code:i32,day:bool){
    match code {
        0 => if day {sun(cx,cy,26,15)} else {moon(cx,cy,30)},
        1|2 => { if day {sun(cx-14,cy-16,18,15)} else {moon(cx-14,cy-16,20)}; fill_circle(cx+8,cy+12,1,0); cloud(cx+8,cy+12,11); }
        3 => cloud(cx,cy,12),
        45|48 => for i in 0..4 { let w=[70,84,62,78][i]; line(cx-w/2,cy-18+i as i32*13,cx+w/2,cy-18+i as i32*13,5,11); },
        71..=77|85|86 => { cloud(cx,cy-12,12); for i in 0..3 { fill_circle(cx-18+i*18,cy+30,4,15); fill_circle(cx-9+i*18,cy+44,4,15);} }
        95..=99 => { cloud(cx,cy-12,11);
            fill_triangle(cx+4,cy+14,cx-12,cy+36,cx+2,cy+34,15); fill_triangle(cx-2,cy+32,cx+12,cy+30,cx-8,cy+54,15); }
        51..=67|80..=82 => { cloud(cx,cy-12,11); for i in 0..3 { line(cx-14+i*16,cy+24,cx-20+i*16,cy+42,3,15); } }
        _ => cloud(cx,cy,12),
    }
}
unsafe fn droplet(cx:i32,cy:i32,ink:i32){ fill_circle(cx,cy+6,9,ink); fill_triangle(cx-8,cy+2,cx+8,cy+2,cx,cy-13,ink); }

// ---------------------------------------------------------------- screens
const TITLES:[&[u8];TABS]=[b"Now",b"Hourly",b"Wind",b"Rain & wind map"];
unsafe fn header(sub:&[u8]){
    let title=TITLES[TAB];
    tx(24,44,300,15,0,0,title);
    let x=24+fw(0,title)+14;
    tx(x,44,536-x,8,0,0,sub);
    for i in 0..TABS { let dx=556+i as i32*20;
        if i==TAB {fill_circle(dx,60,6,15);} else {fill_circle(dx,60,4,5);} }
    content::cursor(556+TAB as i32*20,78,true);
}
unsafe fn footer(hint:&[u8]){ footer_at(24,592,hint); }
unsafe fn footer_at(x:i32,w:i32,hint:&[u8]){
    let mut b=Buf::new();
    if REFRESHING || STATUS_N>0 { b.add(if STATUS_N>0 {&STATUS[..STATUS_N]} else {b"Refreshing..."}); }
    else if CUR_N>0 { b.add(b"Updated ").add(cf(C_UPDATED)); if !hint.is_empty() { b.add(b"  -  ").add(hint); } }
    else { b.add(hint); }
    tx(x,450,w,6,1,0,b.s());
}
unsafe fn waiting(){
    header(b"");
    weather_icon(320,190,2,true);
    let msg:&[u8]=if STATUS_N>0 {&STATUS[..STATUS_N]} else {b"Waiting for your phone..."};
    tx(40,270,560,15,0,1,msg);
    tx(40,312,560,8,1,1,b"Open Sky Glance in the Mira phone app to share your location.");
    footer(b"swipe for tabs");
}

unsafe fn render_now(){
    let mut b=Buf::new();
    header(fit(0,cf(C_LOC),300));
    weather_icon(84,142,ci(C_CODE),ci(C_DAY)!=0);
    big_temp(156,90,ci(C_TEMP));
    // High / low, feels like and humidity column.
    let x=deg(420,86,15,2,b.add(b"H ").num(ci(C_HI)).s()); b.n=0;
    deg(x+10,86,11,2,b.add(b"L ").num(ci(C_LO)).s()); b.n=0;
    deg(420,132,12,0,b.add(b"Feels ").num(ci(C_FEELS)).s()); b.n=0;
    droplet(430,180,15);
    b.add(b"").num(ci(C_HUM)).add(b"%");
    tx(450,162,120,15,2,0,b.s());
    let hx=450+fw(2,b.s())+8; b.n=0;
    tx(hx,175,620-hx,9,1,0,b"humidity");
    // Condition line.
    b.add(cf(C_COND)).add(b"  -  Wind ").num(ci(C_WIND)).add(b" mph ").add(COMPASS[sector(ci(C_WDIR))]);
    if ci(C_GUST)>ci(C_WIND)+4 { b.add(b", gusts ").num(ci(C_GUST)); }
    tx(24,206,592,13,0,0,b.s()); b.n=0;

    // Hour-by-hour rain chance and sunshine strip for the next 12 hours.
    let (x0,cw,bw,bh)=(104,43,29,62);
    let (rain_base,sun_base)=(318,404);
    let count=HRS_N.min(12);
    let (mut pmax,mut pat,mut uvmax)=(0u8,0usize,0u8);
    for i in 0..count { if HRS[i].pop>pmax {pmax=HRS[i].pop;pat=i;} uvmax=uvmax.max(HRS[i].uv); }
    tx(24,252,78,11,1,0,b"Rain");
    if count>0 { b.add(b"max ").num(pmax as i32).add(b"%"); tx(24,276,80,7,1,0,b.s()); b.n=0; }
    tx(24,338,78,11,1,0,b"Sun");
    if count>0 { b.add(b"UV ").num(((uvmax as i32)+5)/10); tx(24,362,80,7,1,0,b.s()); }
    line(x0,rain_base,x0+cw*12,rain_base,1,4);
    line(x0,sun_base,x0+cw*12,sun_base,1,4);
    line(x0,rain_base-bh/2,x0+cw*12,rain_base-bh/2,1,2);
    line(x0,sun_base-bh/2,x0+cw*12,sun_base-bh/2,1,2);
    for i in 0..count {
        let h=&HRS[i]; let x=x0+i as i32*cw+(cw-bw)/2;
        let ph=h.pop as i32*bh/100; if ph>0 { fill_rect(x,rain_base-ph,bw,ph,if i==pat {13} else {9}); }
        let sh=h.sun as i32*bh/100; if sh>0 { fill_rect(x,sun_base-sh,bw,sh,14); }
        if i%3==0 { tx(x0+i as i32*cw-6,412,cw+12,10,1,1,&h.label[..h.ll as usize]); }
    }
    footer(b"tap to refresh");
}

unsafe fn render_hourly(){
    let start=H_PAGE*12;
    header(if H_PAGE==0 {b"next 12 hours"} else {b"hours 13-24"});
    let heads:[(&[u8],i32,i32,i32);7]=[(b"Time",24,72,0),(b"Temp",96,70,2),(b"Rain",176,80,2),(b"Inches",262,96,2),(b"Sun",366,74,2),(b"UV",446,52,2),(b"Wind",506,112,2)];
    for (t,x,w,a) in heads { tx(x,84,w,8,1,a,t); }
    line(24,110,616,110,1,4);
    let mut cols=[Buf::new(),Buf::new(),Buf::new(),Buf::new(),Buf::new(),Buf::new(),Buf::new()];
    let end=(start+12).min(HRS_N);
    for i in start..end {
        let h=&HRS[i];
        if i>start { for c in cols.iter_mut(){c.add(b"\n");} }
        cols[0].add(&h.label[..h.ll as usize]);
        cols[1].num(h.temp as i32);
        cols[2].num(h.pop as i32).add(b"%");
        if h.amt==0 {cols[3].add(b"-");} else {cols[3].hundredths(h.amt);}
        cols[4].num(h.sun as i32).add(b"%");
        cols[5].num((h.uv as i32+5)/10);
        cols[6].num(h.wind as i32).add(b" ").add(COMPASS[sector(h.dir as i32)]);
    }
    if end>start {
        let inks=[15,15,13,13,14,12,12];
        for (k,(_,x,w,a)) in heads.iter().enumerate() { column(*x,114,*w,inks[k],0,*a,25,cols[k].s()); }
    } else { tx(24,200,592,9,0,1,b"No hourly forecast yet"); }
    let mut b=Buf::new();
    b.add(b"Today: sun ").add(cf(C_RISE)).add(b"-").add(cf(C_SET)).add(b"  dew ").num(ci(C_DEW))
     .add(b"  rain ").hundredths(ci(C_RAIN).clamp(0,9999) as u16).add(b" in, ").num(ci(C_POP)).add(b"%");
    tx(24,424,592,9,1,0,b.s());
    footer(if H_PAGE==0 {b"tap for hours 13-24"} else {b"tap for next 12 hours"});
}

unsafe fn render_wind(){
    let mut b=Buf::new();
    header(b"next 12 hours");
    // Compass with the current wind; the arrow points where the wind blows.
    let (cx,cy,r)=(112,186,64);
    stroke_circle(cx,cy,r,2,7);
    for s in [0usize,4,8,12] { let (ux,uy)=(SIN16[s],-SIN16[(s+4)%16]);
        line(cx+ux*(r-10)/1000,cy+uy*(r-10)/1000,cx+ux*r/1000,cy+uy*r/1000,2,9); }
    tx(cx-20,94,40,11,1,1,b"N");
    let wind=ci(C_WIND);
    if wind<2 { stroke_circle(cx,cy,8,2,15); } else { arrow(cx,cy,(sector(ci(C_WDIR))+8)%16,96,15,4); }
    b.num(wind).add(b" mph"); tx(210,92,410,15,3,0,b.s()); b.n=0;
    b.add(b"from ").add(COMPASS[sector(ci(C_WDIR))]); tx(210,150,410,12,0,0,b.s()); b.n=0;
    b.add(b"Gusts ").num(ci(C_GUST)).add(b" mph"); tx(210,184,410,12,0,0,b.s()); b.n=0;
    let count=HRS_N.min(12);
    let (mut peak,mut at)=(0u8,0usize);
    for i in 0..count { let g=HRS[i].gust.max(HRS[i].wind); if g>peak {peak=g;at=i;} }
    if count>0 {
        b.add(b"Peak gust ").num(peak as i32).add(b" mph at ").add(&HRS[at].label[..HRS[at].ll as usize]);
        tx(210,218,410,10,0,0,b.s()); b.n=0;
    }
    // Hourly wind bars with gust ticks and direction arrows.
    let (x0,cw,top,bottom)=(60,46,292,414);
    let scale=((peak as i32+9)/10*10).max(10);
    b.num(scale); tx(4,top-10,50,7,1,2,b.s());
    tx(4,bottom-16,50,7,1,2,b"0");
    line(x0,bottom,x0+cw*12,bottom,1,5);
    line(x0,(top+bottom)/2,x0+cw*12,(top+bottom)/2,1,2);
    for i in 0..count {
        let h=&HRS[i]; let x=x0+i as i32*cw;
        let wh=h.wind as i32*(bottom-top)/scale; if wh>0 { fill_rect(x+9,bottom-wh,cw-18,wh,9); }
        let gy=bottom-h.gust as i32*(bottom-top)/scale; line(x+5,gy,x+cw-5,gy,2,15);
        if h.wind>=2 { arrow(x+cw/2,top-22,(sector(h.dir as i32)+8)%16,22,13,2); } else { stroke_circle(x+cw/2,top-22,4,2,11); }
        if i%3==0 { tx(x-4,418,cw+8,10,1,1,&h.label[..h.ll as usize]); }
    }
    footer(b"bars wind, ticks gusts, mph");
}

const SHADE:[i32;10]=[0,3,4,5,6,7,9,10,12,14];
unsafe fn render_map(){
    let mut b=Buf::new();
    header(b"25 mi radius");
    if M_N==0 { tx(24,220,592,9,0,1,b"Map loading..."); footer(b""); return; }
    let f=M_FRAME.min(M_N-1);
    let (x0,y0,cs)=(20,90,51);
    let size=cs*GRID as i32;
    let (mut wet,mut sum,mut max)=(0,0i32,0u8);
    for i in 0..CELLS {
        let (row,col)=((i/GRID) as i32,(i%GRID) as i32);
        let lvl=M_P[f][i] as usize;
        if lvl>0 { fill_rect(x0+col*cs+1,y0+row*cs+1,cs-2,cs-2,SHADE[lvl]); wet+=1; }
        let s=M_S[f][i]; sum+=s as i32; max=max.max(s);
    }
    stroke_rect(x0,y0,size,size,1,5);
    let (cx,cy)=(x0+size/2,y0+size/2);
    stroke_circle(cx,cy,cs*3,1,6);
    for i in 0..CELLS {
        let (row,col)=((i/GRID) as i32,(i%GRID) as i32);
        let (ax,ay)=(x0+col*cs+cs/2,y0+row*cs+cs/2);
        let ink=if M_P[f][i]>=6 {0} else {15};
        let s=M_S[f][i] as i32;
        if s<2 { fill_circle(ax,ay,2,ink); }
        else { arrow(ax,ay,(M_D[f][i] as usize+8)%16,(12+s*3/2).min(46),ink,2); }
    }
    stroke_circle(cx,cy,9,3,0); stroke_circle(cx,cy,6,2,15);
    tx(x0+4,y0+2,24,9,1,0,b"N");
    // Side panel: time, timeline, legend and summary.
    let px=414;
    tx(px,86,216,15,3,0,&M_LABEL[f][..M_LL[f] as usize]);
    if f==0 { b.add(b"now"); } else { b.add(b"+").num(f as i32).add(if f==1 {b" hour"} else {b" hours"}); }
    tx(px,142,216,10,0,0,b.s()); b.n=0;
    for i in 0..MAP_HOURS { let ink=if i==f {15} else if i<M_N {5} else {2};
        fill_rect(px+i as i32*17,182,13,if i==f {12} else {8},ink); }
    tx(px,200,216,9,1,0,if M_PLAY {b"Playing - tap to pause"} else {b"Paused - tap to play"});
    tx(px,236,216,11,1,0,b"Rain intensity");
    for (k,l) in [1usize,3,5,7,9].iter().enumerate() { fill_rect(px+k as i32*42,262,38,18,SHADE[*l]); }
    stroke_rect(px,262,208,18,1,4);
    tx(px,284,100,8,1,0,b"light"); tx(px+108,284,100,8,1,2,b"heavy");
    b.add(b"Wind avg ").num(sum/CELLS as i32).add(b" mph\nMax ").num(max as i32).add(b" mph\nRain in ").num(wet*100/CELLS as i32).add(b"% of area");
    column(px,320,216,12,1,0,26,b.s());
    tx(px,398,216,7,1,0,b"Arrows: wind heading");
    tx(px,422,216,7,1,0,b"You: center, north up");
    footer_at(px,216,b"");
}

unsafe fn render(){
    DIRTY=false;
    begin();
    if CUR_N==0 && TAB!=3 { waiting(); }
    else { match TAB { 0=>render_now(), 1=>render_hourly(), 2=>render_wind(), _=>render_map() } }
    present();
}

// ---------------------------------------------------------------- exports
#[no_mangle] pub unsafe extern "C" fn app_init(){
    BRIDGE=companion::Companion::new(APP);
    TAB=0; DIRTY=true; NEED_SYNC=true; NEED_REFRESH=false; REFRESHING=false; WAIT_MS=0;
    CUR_N=0; STATUS_N=0; HRS_N=0; H_PAGE=0; M_N=0; M_FRAME=0; M_PLAY=true; M_ACC=0;
    foreground_interval_ms(150);
    render();
}
#[no_mangle] pub unsafe extern "C" fn app_render(ms:i32){
    let ms=ms.clamp(0,5000) as u32;
    if content::context_changed(){DIRTY=true;}
    if let Some(m)=BRIDGE.tick(ms){ handle(m); }
    if NEED_SYNC { if BRIDGE.send(b"{\"type\":\"sync\"}") {NEED_SYNC=false;WAIT_MS=0;} }
    else if NEED_REFRESH { if BRIDGE.send(b"{\"type\":\"refresh\"}") {NEED_REFRESH=false;} }
    // If the phone never answered the first sync, ask again every 8 seconds.
    if CUR_N==0 && !NEED_SYNC { WAIT_MS=WAIT_MS.saturating_add(ms); if WAIT_MS>=8000 {NEED_SYNC=true;} }
    if TAB==3 && M_PLAY && M_N>1 {
        M_ACC+=ms;
        if M_ACC>=1100 { M_ACC=0; M_FRAME=(M_FRAME+1)%M_N; DIRTY=true; }
    }
    if DIRTY {render();}
}
#[no_mangle] pub unsafe extern "C" fn app_event(e:i32){
    match e {
        1 => { TAB=(TAB+TABS-1)%TABS; H_PAGE=0; M_ACC=0; }
        2 => { TAB=(TAB+1)%TABS; H_PAGE=0; M_ACC=0; }
        0 => match TAB {
            1 => H_PAGE^=1,
            3 => { M_PLAY=!M_PLAY; M_ACC=0; }
            _ => { NEED_REFRESH=true; REFRESHING=true; STATUS_N=0; }
        },
        _ => return,
    }
    render();
}
#[no_mangle] pub extern "C" fn app_stop() {}
