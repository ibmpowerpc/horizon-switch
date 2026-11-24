#include "column_mesh.h"
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
_Static_assert(sizeof(struct cm_info) == 72, "column mesh info ABI");
static uint32_t word(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1]<<8 | (uint32_t)p[2]<<16 | (uint32_t)p[3]<<24;
}
static uint8_t *read_file(const char *path, size_t *length) {
    FILE *file=fopen(path,"rb"); assert(file);
    assert(fseek(file,0,SEEK_END)==0); long size=ftell(file);
    assert(size>=48 && size<=16*1024*1024 && fseek(file,0,SEEK_SET)==0);
    uint8_t *bytes=malloc((size_t)size); assert(bytes);
    assert(fread(bytes,1,(size_t)size,file)==(size_t)size && !ferror(file));
    assert(fclose(file)==0); *length=(size_t)size; return bytes;
}
static int all_bytes(const void *p,size_t n,uint8_t value) {
    const uint8_t *b=p; for(size_t i=0;i<n;i++)if(b[i]!=value)return 0; return 1;
}
static uint64_t fingerprint(const void *p,size_t n) {
    const uint8_t *b=p; uint64_t h=UINT64_C(14695981039346656037);
    for(size_t i=0;i<n;i++){h^=b[i];h*=UINT64_C(1099511628211);} return h;
}
int main(int argc,char **argv) {
    assert(argc==4); size_t cl,bl,rl;
    uint8_t *capture=read_file(argv[1],&cl),*bank=read_file(argv[2],&bl),*ref=read_file(argv[3],&rl);
    assert(!memcmp(ref,"CTTEX001",8));
    uint32_t count=word(ref+16),draw_count=word(ref+20);
    assert(count>0 && count<262140 && draw_count>0 && draw_count<256);
    struct cm_info query,info;
    memset(&query,0xA5,sizeof(query));
    assert(craftrustnx_column_mesh(capture,cl,bank,bl,8,NULL,0,NULL,0,&query)==0);
    assert(query.scene.mesh.vertex_count==count && query.scene.draw_count==draw_count);
    assert(query.section_index==8 && query.section_min_y==64);
    assert(query.cells_with_geometry==876 && query.empty_cells==3220);
    assert(query.scene.width==word(ref+24) && query.scene.height==word(ref+28));
    assert(!memcmp(query.scene.mesh.min,ref+40,24));
    size_t vertex_bytes=(size_t)count*sizeof(struct tx_vertex),draw_bytes=(size_t)draw_count*sizeof(struct tx_draw);
    struct tx_vertex *vertices=malloc(vertex_bytes+sizeof(*vertices));
    struct tx_draw *draws=malloc(draw_bytes+sizeof(*draws)); assert(vertices&&draws);
    memset(vertices,0xA5,vertex_bytes+sizeof(*vertices)); memset(draws,0xA5,draw_bytes+sizeof(*draws));
    assert(craftrustnx_column_mesh(capture,cl,bank,bl,8,vertices,count+1,draws,draw_count+1,&info)==0);
    assert(!memcmp(&info,&query,sizeof(info)));
    assert(!memcmp(vertices,ref+64,vertex_bytes));
    assert(!memcmp(draws,ref+64+vertex_bytes,draw_bytes));
    assert(all_bytes(vertices+count,sizeof(*vertices),0xA5));
    assert(all_bytes(draws+draw_count,sizeof(*draws),0xA5));
    assert(info.scene.atlas_offset+info.scene.atlas_bytes==bl);
    assert(64+vertex_bytes+draw_bytes+info.scene.atlas_bytes==rl);
    assert(!memcmp(bank+info.scene.atlas_offset,ref+64+vertex_bytes+draw_bytes,info.scene.atlas_bytes));
    printf("real column mesh: byte-identical to 0.11, vertices=%u draws=%u fingerprint=%016" PRIx64 "\n",count,draw_count,fingerprint(vertices,vertex_bytes));

    /* Real late failures must not leave a partially emitted mesh. */
    for(int which=0;which<6;which++) {
        memset(vertices,0xA5,vertex_bytes+sizeof(*vertices)); memset(draws,0xA5,draw_bytes+sizeof(*draws));
        memset(&info,0xA5,sizeof(info));
        uint32_t cap=count;size_t capture_length=cl,bank_length=bl;
        uint8_t old=bank[40];
        uint8_t saved[4]; size_t cell=48+8*4096*4;memcpy(saved,capture+cell,4);
        if(which==0)cap=count-1;
        if(which==1)capture_length--;
        if(which==2)bank_length--;
        if(which==3)bank[40]=1;
        if(which==4)memset(capture+cell,0xFF,4);
        if(which==5)bank_length=16*1024*1024+1;
        assert(craftrustnx_column_mesh(capture,capture_length,bank,bank_length,8,vertices,cap,draws,draw_count,&info)<0);
        bank[40]=old;memcpy(capture+cell,saved,4);
        assert(all_bytes(&info,sizeof(info),0));
        assert(all_bytes(vertices,vertex_bytes+sizeof(*vertices),0xA5));
        assert(all_bytes(draws,draw_bytes+sizeof(*draws),0xA5));
    }
    memset(&info,0xA5,sizeof(info));
    assert(craftrustnx_column_mesh(capture,cl,bank,bl,8,NULL,0,draws,draw_count,&info)==-1);
    assert(all_bytes(&info,sizeof(info),0));
    free(vertices);free(draws);free(capture);free(bank);free(ref);
    puts("column mesh query/build, real reference, transactional failures PASS");return 0;
}
